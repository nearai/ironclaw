//! The caller's per-channel connection surface.
//!
//! **Why here.** [`ChannelAuthAccountState`] is literally the argument pair of
//! [`crate::project_auth_account_state`] — two of this crate's own §6.3 enums —
//! and [`ChannelConnectionService`] is the port that produces it. The port's
//! only other vocabulary is `ironclaw_product_contracts::surface`, which this
//! crate may name (a `substrates -> contracts` edge, the same downward shape
//! `ironclaw_attachments` already carries for its landing ports).
//!
//! The projection decision does **not** cross the port: `project_auth_account_state`
//! stays a pure function of the two enums and is called by `ironclaw_product`'s
//! extensions wire, so nothing authoritative moves with the vocabulary.

use async_trait::async_trait;
use ironclaw_product_contracts::surface::{ProductSurfaceCaller, ProductSurfaceError};

use crate::credential::CredentialAccountStatus;
use crate::flow::AuthFlowStatus;

/// The caller's durable auth-account signal for one channel extension's vendor
/// — the raw inputs the extensions-list service feeds to
/// [`crate::project_auth_account_state`] so an account renders its real
/// §6.3 state (`expired` / `refresh-failed` / `authenticating`) plus a typed
/// last error, instead of the connected/disconnected collapse the
/// [`ChannelConnectionService::caller_channel_connections`] bool alone permits.
///
/// Both inputs are optional. A service that only knows the caller holds a live
/// grant leaves both `None` and the projection falls back to the connection
/// bool (a live grant backfills to `connected`, MIG-1); a service that reads the
/// durable credential-account status supplies `account_status` (and, mid-flow,
/// `active_flow_status`) so the wire surfaces the real state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelAuthAccountState {
    /// The caller's durable credential-account status for the extension's
    /// vendor, when the service can read it.
    pub account_status: Option<CredentialAccountStatus>,
    /// A live (non-terminal) auth flow for the extension's vendor, when one is
    /// in progress — projects to `authenticating`.
    pub active_flow_status: Option<AuthFlowStatus>,
}

/// Per-user channel connection state. Returns, for the calling user, which
/// channel extensions they have personally connected — a per-user vendor OAuth
/// grant, typically. Keyed by channel package id -> `true` when connected.
/// Only channels that have a per-user connection concept appear in the map;
/// absence means "no per-user connection concept for this channel".
#[async_trait]
pub trait ChannelConnectionService: Send + Sync {
    async fn caller_channel_connections(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<std::collections::HashMap<String, bool>, ProductSurfaceError>;

    /// The caller's durable auth-account signal per channel extension, keyed by
    /// channel package id — richer than the connected/disconnected bool
    /// [`Self::caller_channel_connections`] returns. Lets the extensions wire
    /// project the shared §6.3 auth-account state (`expired` / `refresh-failed`)
    /// and its typed last error for each vendor account.
    ///
    /// Default: empty. A service that does not yet read durable credential-account
    /// status reports none and the wire falls back to the connection bool; the
    /// production channel-connection service overrides this to project each
    /// caller's account status.
    async fn caller_channel_account_states(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<std::collections::HashMap<String, ChannelAuthAccountState>, ProductSurfaceError>
    {
        Ok(std::collections::HashMap::new())
    }

    async fn disconnect_channel_for_caller(
        &self,
        _caller: ProductSurfaceCaller,
        _channel: &str,
    ) -> Result<(), ProductSurfaceError> {
        Err(ProductSurfaceError::service_unavailable(false))
    }
}
