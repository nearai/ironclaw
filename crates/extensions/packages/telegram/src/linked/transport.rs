//! **The only module in this package that may open a socket.**
//!
//! Everything MTProto reaches Telegram through [`MtprotoConnection`]. No other
//! file here names `SenderPool`, `Client::new`, or `tokio::spawn`, and that
//! confinement is one of PROPOSAL §3.4's compensating controls: MTProto
//! bypasses the manifest egress allowlist, the SSRF checks, the response caps,
//! and host credential injection, so "where can this package dial from?" has
//! to have a one-file answer. Address *policy* still lives one layer down, in
//! [`crate::linked::session_store`], because `Session::dc_option` is the only
//! seam grammers consults on its dial path.
//!
//! # Two non-obvious requirements, both load-bearing
//!
//! 1. **The runner must be spawned.** `SenderPool::new` builds a runner, a
//!    handle, and an update channel; the handle only queues requests. Without
//!    `tokio::spawn(runner.run())` every `invoke` parks forever on a oneshot
//!    that nothing will ever complete.
//! 2. **The update receiver must be dropped.** The channel is *unbounded*, and
//!    the client-side `update_queue_limit` bounds `UpdateStream`'s internal
//!    deque, not this channel — so holding the receiver without draining it
//!    leaks for the life of the session. Dropping it is safe and verified: both
//!    send sites discard their result (`let _ = updates.send(…)`), and no code
//!    path couples update delivery to connection health or request processing.
//!    Reads are live in this design (PROPOSAL §3.1), so nothing wants updates.
//!
//! # Retries live here, not in the client
//!
//! grammers is configured with [`NoRetries`]. The default `AutoSleep` policy
//! retries once on **any** `Io` error, writes included, and no custom policy
//! can fix that: `RetryContext` carries only `fail_count`, `slept_so_far`, and
//! the error, and the request's constructor id is available solely via
//! `RpcError.caused_by` — absent for exactly the `Io`/`Dropped` cases where
//! double-send risk lives. Read/write discrimination is therefore impossible at
//! the policy layer and is expressed here instead, as an explicit
//! [`VendorOpKind`] on every call.

use std::sync::Arc;
use std::time::Duration;

use grammers_client::client::{ClientConfiguration, NoRetries};
use grammers_client::sender::{ConnectionParams, RpcError};
use grammers_client::{Client, InvocationError, SenderPool};
use grammers_tl_types as tl;
use tokio::task::JoinHandle;

use crate::linked::session_store::IronclawSession;

/// What Telegram shows the user in *Settings → Devices*. A recognizable name is
/// a product requirement, not decoration: PROPOSAL §3.2 leans on the user being
/// able to *see* and revoke an unexpected device.
const DEVICE_MODEL: &str = "IronClaw";

/// Attempts a **read** gets before it gives up. Writes get one, always.
const MAX_READ_ATTEMPTS: u32 = 3;

/// Base delay between read retries; doubled per attempt.
const READ_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);

/// The longest flood wait this wrapper will sit through before surfacing the
/// rate limit to the caller. Beyond it, a caller (and a user) is better served
/// by an explicit "rate limited" than by a stalled call.
const MAX_FLOOD_WAIT: Duration = Duration::from_secs(30);

/// Telegram's rate-limit status code.
const FLOOD_WAIT_CODE: i32 = 420;

/// Whether a request, if repeated, would repeat a side effect.
///
/// This is the distinction the retry policy layer structurally cannot make, so
/// every call site states it. There is no default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VendorOpKind {
    /// Safe to repeat: repeating it produces the same answer, not a second
    /// effect.
    Read,
    /// Not safe to repeat. A send, edit, delete, or reaction.
    Write,
}

/// Typed transport failures.
///
/// The distinction that matters is [`TransportError::OutcomeUnknown`]: it is
/// **not** a failure. It means the request may well have executed and this
/// process cannot tell — which must surface as sent-unverified evidence
/// (PROPOSAL §6.2), never as an error that invites the model to send again.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TransportError {
    /// Telegram processed the request and rejected it. The name is the vendor's
    /// screaming-snake-case code (`FLOOD_WAIT`, `AUTH_KEY_UNREGISTERED`); it
    /// carries no user content.
    #[error("telegram rejected the request: {name}")]
    Rpc {
        code: i32,
        name: String,
        value: Option<u32>,
    },
    /// The request may or may not have executed. Only ever produced for a
    /// [`VendorOpKind::Write`].
    #[error("telegram request outcome is unknown — it may have been executed")]
    OutcomeUnknown,
    /// The sender-pool runner is gone. The connection is unusable and the
    /// pooled entry has to be rebuilt from the stored blob.
    #[error("the mtproto runner is no longer running")]
    RunnerGone,
    /// Session custody failed underneath a request.
    #[error("linked-session custody failed during a telegram request")]
    Session,
    /// The transport failed and retries (if any were allowed) are exhausted.
    #[error("telegram transport is unavailable")]
    Unavailable,
    /// The session does not know the datacenter the request needed.
    #[error("telegram datacenter is not known to this session")]
    InvalidDc,
}

impl TransportError {
    /// The vendor error name, when Telegram produced one.
    pub(crate) fn rpc_name(&self) -> Option<&str> {
        match self {
            Self::Rpc { name, .. } => Some(name.as_str()),
            _ => None,
        }
    }
}

impl From<&RpcError> for TransportError {
    fn from(error: &RpcError) -> Self {
        Self::Rpc {
            code: error.code,
            name: error.name.clone(),
            value: error.value,
        }
    }
}

/// One live MTProto connection: a driven sender-pool runner, a client on top of
/// it, and the session both share.
///
/// Dropping this aborts the runner, which drops every socket the pool held.
pub(crate) struct MtprotoConnection {
    client: Client,
    session: Arc<IronclawSession>,
    runner: JoinHandle<()>,
}

impl MtprotoConnection {
    /// Build a connection. **Performs no I/O**: grammers connects lazily, on
    /// the first request to a datacenter, which is what lets binding stay
    /// contractually I/O-free.
    ///
    /// Must be called from inside a Tokio runtime — it spawns the runner task.
    pub(crate) fn open(session: Arc<IronclawSession>, api_id: i32) -> Self {
        let SenderPool {
            runner,
            handle,
            updates,
        } = SenderPool::with_configuration(Arc::clone(&session), api_id, connection_params());

        // REQUIRED. See this module's header: without a driven runner, every
        // invoke parks forever.
        let runner = tokio::spawn(runner.run());

        // REQUIRED, and the global rule for every client this package builds:
        // the channel is unbounded and nothing consumes updates.
        drop(updates);

        let client = Client::with_configuration(
            handle,
            ClientConfiguration {
                retry_policy: Box::new(NoRetries),
                // Left on: it is what populates the peer cache the session then
                // bounds (MAX_PEER_CACHE_ENTRIES). Turning it off would trade a
                // bounded cache for a flood-prone `resolve_peer` on every call.
                auto_cache_peers: true,
            },
        );

        Self {
            client,
            session,
            runner,
        }
    }

    /// The underlying client, for the few grammers high-level flows the raw
    /// invoke path cannot express (`sign_in`, `check_password`).
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// The session this connection reads and persists through.
    pub(crate) fn session(&self) -> &Arc<IronclawSession> {
        &self.session
    }

    /// Whether the runner task is still alive.
    ///
    /// This is what separates §7.1's two readings of `InvocationError::Dropped`
    /// — "the runner is gone, rebuild" versus "a connection-lifecycle race with
    /// a healthy runner" — which the error value alone cannot distinguish.
    pub(crate) fn is_runner_alive(&self) -> bool {
        !self.runner.is_finished()
    }

    /// Invoke a request against the session's home datacenter.
    pub(crate) async fn invoke<R: tl::RemoteCall>(
        &self,
        request: &R,
        kind: VendorOpKind,
    ) -> Result<R::Return, TransportError> {
        self.dispatch(kind, || self.client.invoke(request)).await
    }

    /// Invoke a request against a named datacenter.
    ///
    /// Needed by the login flow's `MigrateTo` step, which must import the
    /// exported token in the datacenter Telegram named rather than the one the
    /// session currently calls home.
    pub(crate) async fn invoke_in_dc<R: tl::RemoteCall>(
        &self,
        dc_id: i32,
        request: &R,
        kind: VendorOpKind,
    ) -> Result<R::Return, TransportError> {
        self.dispatch(kind, || self.client.invoke_in_dc(dc_id, request))
            .await
    }

    /// The retry loop. Reads may be repeated; writes never are.
    async fn dispatch<T, F, Fut>(
        &self,
        kind: VendorOpKind,
        mut call: F,
    ) -> Result<T, TransportError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, InvocationError>>,
    {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let error = match call().await {
                Ok(value) => return Ok(value),
                Err(error) => error,
            };

            match classify(&error, kind, self.is_runner_alive()) {
                Disposition::Fail(error) => return Err(error),
                Disposition::RetryAfter(delay) => {
                    if attempt >= MAX_READ_ATTEMPTS {
                        return Err(TransportError::Unavailable);
                    }
                    tokio::time::sleep(delay).await;
                }
                Disposition::FloodWait(delay) => {
                    if delay > MAX_FLOOD_WAIT || attempt >= MAX_READ_ATTEMPTS {
                        return Err(TransportError::from(&flood_error(delay)));
                    }
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

/// Decide what one failure means, given what kind of operation produced it and
/// whether the runner is still alive.
///
/// A free function on purpose: this is the rule the whole module exists to
/// enforce, and a rule that can only be exercised through a live socket is a
/// rule nothing checks.
fn classify(error: &InvocationError, kind: VendorOpKind, runner_alive: bool) -> Disposition {
    match error {
        InvocationError::Rpc(rpc) => {
            // A flood wait is the one RPC error worth waiting out, and only
            // for a read: repeating a write after a flood wait would repeat
            // the effect if the first attempt actually landed.
            if rpc.code == FLOOD_WAIT_CODE
                && kind == VendorOpKind::Read
                && let Some(seconds) = rpc.value
            {
                return Disposition::FloodWait(Duration::from_secs(u64::from(seconds)));
            }
            // Every other RPC error means the server processed the request
            // and refused it. Repeating it repeats the refusal.
            Disposition::Fail(TransportError::from(rpc))
        }
        InvocationError::Session(_) => Disposition::Fail(TransportError::Session),
        InvocationError::InvalidDc => Disposition::Fail(TransportError::InvalidDc),
        InvocationError::Authentication(_) => Disposition::Fail(TransportError::Unavailable),
        // Io / Dropped / Transport / Deserialize: the request may have
        // reached the wire. For a write that is `OutcomeUnknown` — never
        // "not executed" — and it must never be retried.
        InvocationError::Io(_)
        | InvocationError::Transport(_)
        | InvocationError::Deserialize(_)
        | InvocationError::Dropped => {
            if !runner_alive {
                return Disposition::Fail(TransportError::RunnerGone);
            }
            match kind {
                VendorOpKind::Write => Disposition::Fail(TransportError::OutcomeUnknown),
                VendorOpKind::Read => Disposition::RetryAfter(READ_RETRY_BASE_DELAY),
            }
        }
    }
}

impl Drop for MtprotoConnection {
    /// Aborts the runner, which drops every socket it held.
    ///
    /// Deliberately does **not** flush the session: `Drop` is synchronous and a
    /// flush is asynchronous, so a "best-effort flush here" would be a
    /// `tokio::spawn` racing shutdown that silently does nothing. Callers flush
    /// explicitly before dropping (`SessionPool::evict`), the same reasoning
    /// that keeps `auth.logOut` out of the pending-link `Drop` (PROPOSAL §4.3).
    fn drop(&mut self) {
        // Asks the runner to close its connections gracefully; the abort below
        // is what guarantees the task ends either way.
        self.client.disconnect();
        self.runner.abort();
    }
}

/// What to do about one failed attempt.
#[derive(Debug)]
enum Disposition {
    Fail(TransportError),
    RetryAfter(Duration),
    FloodWait(Duration),
}

/// Rebuild the flood-wait RPC error for a wait this wrapper declined to sit
/// through, so the caller sees the vendor's own code rather than a generic
/// unavailability.
fn flood_error(delay: Duration) -> RpcError {
    RpcError {
        code: FLOOD_WAIT_CODE,
        name: "FLOOD_WAIT".to_string(),
        value: u32::try_from(delay.as_secs()).ok(),
        caused_by: None,
    }
}

/// What Telegram is told about this client on every new connection.
fn connection_params() -> ConnectionParams {
    ConnectionParams {
        device_model: DEVICE_MODEL.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_model_is_recognizable_to_a_user_revoking_it() {
        assert_eq!(connection_params().device_model, DEVICE_MODEL);
    }

    #[test]
    fn an_rpc_error_projects_without_carrying_a_message_body() {
        let error = TransportError::from(&RpcError {
            code: 400,
            name: "PHONE_CODE_INVALID".to_string(),
            value: None,
            caused_by: None,
        });
        assert_eq!(error.rpc_name(), Some("PHONE_CODE_INVALID"));
    }

    #[test]
    fn an_unknown_outcome_is_its_own_variant_not_a_generic_failure() {
        // Reconnecting is safe; *retrying the write* is not, and collapsing
        // `OutcomeUnknown` into `Unavailable` is exactly how a duplicate send
        // ships — a caller that cannot tell them apart will retry both.
        assert!(matches!(
            TransportError::OutcomeUnknown,
            TransportError::OutcomeUnknown
        ));
        assert_eq!(TransportError::OutcomeUnknown.rpc_name(), None);
        assert_eq!(TransportError::RunnerGone.rpc_name(), None);
    }

    #[test]
    fn a_declined_flood_wait_keeps_the_vendor_code() {
        let error = TransportError::from(&flood_error(Duration::from_secs(90)));
        assert_eq!(error.rpc_name(), Some("FLOOD_WAIT"));
        match error {
            TransportError::Rpc { code, value, .. } => {
                assert_eq!(code, FLOOD_WAIT_CODE);
                assert_eq!(value, Some(90));
            }
            other => panic!("expected an rpc error, got {other:?}"),
        }
    }

    fn rpc(code: i32, name: &str, value: Option<u32>) -> InvocationError {
        InvocationError::Rpc(RpcError {
            code,
            name: name.to_string(),
            value,
            caused_by: None,
        })
    }

    fn dropped() -> InvocationError {
        InvocationError::Dropped
    }

    fn io() -> InvocationError {
        InvocationError::Io(std::io::Error::other("connection reset"))
    }

    /// The rule this whole module exists for: a write whose outcome is unknown
    /// must never be retried, and must never be reported as "did not happen".
    #[test]
    fn a_write_with_an_unknown_outcome_is_never_retried() {
        for error in [dropped(), io()] {
            match classify(&error, VendorOpKind::Write, true) {
                Disposition::Fail(TransportError::OutcomeUnknown) => {}
                other => panic!("a write must surface as OutcomeUnknown, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_same_failure_on_a_read_is_retried_instead() {
        for error in [dropped(), io()] {
            assert!(
                matches!(
                    classify(&error, VendorOpKind::Read, true),
                    Disposition::RetryAfter(_)
                ),
                "a read is safe to repeat, so it should not fail on the first fault"
            );
        }
    }

    #[test]
    fn a_dead_runner_outranks_the_operation_kind() {
        // `Dropped` alone cannot tell "the runner is gone, rebuild" from "a
        // connection-lifecycle race with a healthy runner" — the liveness of
        // the runner task is what separates them.
        for kind in [VendorOpKind::Read, VendorOpKind::Write] {
            assert!(matches!(
                classify(&dropped(), kind, false),
                Disposition::Fail(TransportError::RunnerGone)
            ));
        }
    }

    #[test]
    fn a_flood_wait_is_slept_on_for_reads_and_surfaced_for_writes() {
        let error = rpc(FLOOD_WAIT_CODE, "FLOOD_WAIT", Some(5));
        assert!(matches!(
            classify(&error, VendorOpKind::Read, true),
            Disposition::FloodWait(delay) if delay == Duration::from_secs(5)
        ));
        // Sleeping then re-sending a write would repeat the effect if the first
        // attempt actually landed before the limit tripped.
        assert!(matches!(
            classify(&error, VendorOpKind::Write, true),
            Disposition::Fail(TransportError::Rpc { .. })
        ));
    }

    #[test]
    fn a_processed_and_refused_request_is_never_repeated() {
        // An RPC error means the server saw the request and said no. Repeating
        // it repeats the refusal and burns the flood budget.
        for kind in [VendorOpKind::Read, VendorOpKind::Write] {
            assert!(matches!(
                classify(&rpc(400, "PHONE_CODE_INVALID", None), kind, true),
                Disposition::Fail(TransportError::Rpc { .. })
            ));
        }
    }
}
