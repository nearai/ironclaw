//! [`TelegramDeviceLinkAdapter`] — the vendor half of a device-link flow.
//!
//! The host owns the state machine, the revision compare-and-swap, the TTLs,
//! the rate limits, and the credential lifecycle. This module owns exactly the
//! protocol conversation, plus the connection each in-progress login is bound
//! to between UI requests.
//!
//! # Why a login has to be parked
//!
//! A Telegram login is bound to the connected session and datacenter that
//! started it, so the `Client` and its runner task must survive across
//! requests. (Not the intermediate *tokens* — `PasswordToken` has a public
//! constructor and has to be re-derived from fresh SRP parameters anyway.)
//! Hence [`PendingLink`]: bounded, TTL'd, and holding a per-link async mutex
//! that serializes **every** vendor call for that link. The host's revision CAS
//! serializes flow-record writes only; it does not protect a parked client, and
//! a poll overlapping a password submit is a when-not-if race.
//!
//! # QR acceptance is poll-driven, and that is settled
//!
//! Re-export *is* the acceptance mechanism: `auth.loginTokenSuccess` is
//! returned by the export call itself, and `updateLoginToken` merely tells
//! event-driven clients to call it sooner. Telegram Web K — an official client
//! — consumes no updates at all and polls `exportLoginToken` every 3 s. So the
//! recipe here is: poll on the **same** session with identical `except_ids`, at
//! `min(3s, expires − serverNow)`, repaint only when the token bytes change,
//! and correct for **server** time. That is what keeps `drop(updates)` a global
//! rule (PROPOSAL §4.2).
//!
//! # Logout on abort, never in `Drop`
//!
//! Once Telegram has authorized the device, walking away leaves a live
//! authorization the host has forgotten. Every abort path — TTL reap, cancel,
//! shutdown — therefore *awaits* [`abandon`], which calls `auth.logOut`. It
//! cannot live in `Drop`: `Drop` is synchronous, `logOut` is not, and the same
//! value aborts its runner on drop, so a `tokio::spawn` there would be a race
//! with shutdown that silently does nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL;
use grammers_client::client::{LoginToken, PasswordToken, SignInError};
use grammers_session::Session as _;
use grammers_session::types::PeerInfo;
use grammers_tl_types as tl;
use ironclaw_extension_contracts::device_link::{
    DeviceLinkAdapter, DeviceLinkContext, DeviceLinkDisplayKind, DeviceLinkError,
    DeviceLinkErrorCode, DeviceLinkFlowId, DeviceLinkInput, DeviceLinkInputKind, DeviceLinkMode,
    DeviceLinkPayload, DeviceLinkStep,
};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::debug;

use crate::linked::pool::{LinkedAccountRevoker, RevokeOutcome, SessionPool};
use crate::linked::session_store::IronclawSession;
use crate::linked::transport::{MtprotoConnection, VendorOpKind};
use crate::linked::{MAX_PENDING_LINKS, PENDING_LINK_TTL};

mod errors;

use errors::{custody_error, fatal_step, invocation_error, vendor_error};

/// Longest gap between two `exportLoginToken` polls.
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Shortest gap, so a token that is about to expire cannot spin the caller.
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// First backoff after a failed export; doubles up to [`MAX_EXPORT_BACKOFF`].
const INITIAL_EXPORT_BACKOFF: Duration = Duration::from_secs(1);
/// TDLib's defensive ceiling. No flood limit is documented for this method, so
/// the backoff is about not hammering a broken server, not about a known quota.
const MAX_EXPORT_BACKOFF: Duration = Duration::from_secs(60);
/// How many `MigrateTo` hops one login may take before it is a loop.
const MAX_MIGRATIONS: u8 = 4;
/// Code/password submissions one flow gets. An unbounded `check_password`
/// retry is an account-lockout vector, and an unbounded code retry burns the
/// flood budget.
const MAX_INPUT_ATTEMPTS: u8 = 5;
/// How long an abort waits for `auth.logOut` before giving up on it.
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);

/// The vendor half of Telegram's device link.
pub struct TelegramDeviceLinkAdapter {
    api_id: i32,
    api_hash: SecretString,
    revoker: LinkedAccountRevoker,
    pending: PendingLinks,
}

impl TelegramDeviceLinkAdapter {
    /// Build the adapter.
    ///
    /// `api_id`/`api_hash` are the *developer application's* credentials from
    /// `[admin_configuration]`, not the user's — `api_hash` is declared
    /// `secret = true` because Telegram treats it as one. They arrive at
    /// construction rather than through [`DeviceLinkContext::config`], which
    /// carries non-secret values only.
    ///
    /// The pool is borrowed to mint a **narrow** revoke handle; the adapter
    /// never holds the pool itself. It runs inside an auth flow with no
    /// capability authorization, no approval, and no origin gate, so a handle
    /// to every user's live authenticated client is exactly what it must not
    /// have (PROPOSAL §3.3).
    pub fn new(api_id: i32, api_hash: SecretString, pool: &SessionPool) -> Self {
        Self {
            api_id,
            api_hash,
            revoker: pool.revoker(),
            pending: PendingLinks::default(),
        }
    }

    /// Abort every parked link, logging out any that Telegram already
    /// authorized. Called on shutdown, within a bounded grace period.
    pub async fn shutdown(&self) {
        for link in self.pending.drain() {
            abandon(link).await;
        }
    }

    /// Reap links that outlived [`PENDING_LINK_TTL`], logging each out first.
    ///
    /// Deliberately not a background task: reaping is an *async* operation
    /// (it may have to log out), and driving it from the entry points keeps the
    /// abort path on the same thread of control as the flow it aborts.
    async fn reap_expired(&self) {
        for link in self.pending.take_expired() {
            debug!("reaping an expired telegram device link");
            abandon(link).await;
        }
    }

    /// Start the scannable-code path.
    async fn begin_scan(
        &self,
        ctx: &DeviceLinkContext<'_>,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let link = self.park(ctx.flow_id)?;
        let mut state = link.state.lock().await;
        state.server_offset = server_offset(&link.connection).await;
        self.drive_scan(ctx, &link, &mut state).await
    }

    /// Start the phone-number path. No vendor call yet — the first one needs a
    /// number, and issuing login codes is the exact thing that must be
    /// rate-limited host-side before it reaches Telegram.
    async fn begin_identifier(
        &self,
        ctx: &DeviceLinkContext<'_>,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let link = self.park(ctx.flow_id)?;
        let mut state = link.state.lock().await;
        state.phase = PendingPhase::AwaitingIdentifier;
        Ok(identifier_prompt())
    }

    /// Allocate and register a parked link, refusing past the bound.
    fn park(&self, flow_id: &DeviceLinkFlowId) -> Result<Arc<PendingLink>, DeviceLinkError> {
        // Checked before the connection is built, so a pool at capacity does
        // not spawn a runner task only to abort it one line later. `insert`
        // re-checks under the lock; this is the cheap path, not the guarantee.
        self.pending.check_capacity(flow_id)?;
        let session = IronclawSession::in_memory();
        let link = Arc::new(PendingLink {
            connection: MtprotoConnection::open(session, self.api_id),
            state: tokio::sync::Mutex::new(PendingState::default()),
            created_at: Instant::now(),
        });
        self.pending.insert(flow_id.clone(), Arc::clone(&link))?;
        Ok(link)
    }

    /// One export-and-classify round of the scannable-code path.
    async fn drive_scan(
        &self,
        ctx: &DeviceLinkContext<'_>,
        link: &PendingLink,
        state: &mut PendingState,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let request = tl::functions::auth::ExportLoginToken {
            api_id: self.api_id,
            api_hash: self.api_hash.expose_secret().to_string(),
            // Identical on every poll, by contract: a changed set mints a new
            // token and invalidates a scan already in progress.
            except_ids: Vec::new(),
        };
        match link.connection.invoke(&request, VendorOpKind::Read).await {
            Ok(token) => self.apply_login_token(ctx, link, state, token).await,
            // 2FA surfaces on the export call itself, not as a separate step.
            Err(error) if error.rpc_name() == Some("SESSION_PASSWORD_NEEDED") => {
                let token = fetch_password_token(link).await?;
                Ok(self.ask_for_password(state, token))
            }
            Err(error) => {
                let retry_in = state.export_backoff;
                state.export_backoff = (state.export_backoff * 2).min(MAX_EXPORT_BACKOFF);
                if let Some(fatal) = fatal_step(&error) {
                    return Ok(fatal);
                }
                debug!("telegram login-token export failed; backing off");
                Ok(DeviceLinkStep::AwaitingVendor { retry_in })
            }
        }
    }

    /// Interpret one `auth.LoginToken`, following datacenter migrations.
    ///
    /// A loop rather than recursion: an `async fn` that calls itself needs its
    /// recursive future boxed, and a bounded loop says "migrations are finite"
    /// more plainly than a boxed self-call would.
    async fn apply_login_token(
        &self,
        ctx: &DeviceLinkContext<'_>,
        link: &PendingLink,
        state: &mut PendingState,
        token: tl::enums::auth::LoginToken,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let mut token = token;
        for _ in 0..MAX_MIGRATIONS {
            match token {
                tl::enums::auth::LoginToken::Token(exported) => {
                    return Ok(self.paint_token(state, exported));
                }
                tl::enums::auth::LoginToken::Success(success) => {
                    return self
                        .complete_raw(ctx, link, state, success.authorization)
                        .await;
                }
                tl::enums::auth::LoginToken::MigrateTo(migrate) => {
                    let imported = link
                        .connection
                        .invoke_in_dc(
                            migrate.dc_id,
                            &tl::functions::auth::ImportLoginToken {
                                token: migrate.token,
                            },
                            VendorOpKind::Read,
                        )
                        .await
                        .map_err(vendor_error)?;
                    // Persist the move only after the import succeeded: a home
                    // DC pointing at a datacenter that never accepted us is a
                    // session that reconnects to the wrong place forever.
                    link.connection
                        .session()
                        .set_home_dc_id(migrate.dc_id)
                        .await
                        .map_err(custody_error)?;
                    token = imported;
                }
            }
        }
        Err(DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::VendorUnavailable,
            restartable: true,
        })
    }

    /// Record a freshly exported token and decide whether the card repaints.
    ///
    /// Repaint **only** when the bytes change: within the token's window the
    /// server returns the same bytes, so repainting every poll would churn the
    /// code under a user mid-scan.
    fn paint_token(
        &self,
        state: &mut PendingState,
        exported: tl::types::auth::LoginToken,
    ) -> DeviceLinkStep {
        let changed = !matches!(
            &state.phase,
            PendingPhase::AwaitingScan { token } if token == &exported.token
        );
        let remaining = remaining_for(exported.expires, state.server_offset);
        state.export_backoff = INITIAL_EXPORT_BACKOFF;
        state.phase = PendingPhase::AwaitingScan {
            token: exported.token.clone(),
        };

        if !changed {
            return DeviceLinkStep::AwaitingVendor {
                retry_in: poll_interval(remaining),
            };
        }
        match login_payload(&exported.token) {
            Ok(payload) => DeviceLinkStep::Display {
                kind: DeviceLinkDisplayKind::QrCode,
                payload,
                expires_in: remaining,
            },
            Err(step) => step,
        }
    }

    /// Move to the password phase and ask for it.
    fn ask_for_password(&self, state: &mut PendingState, token: PasswordToken) -> DeviceLinkStep {
        let step = password_prompt(&token);
        state.phase = PendingPhase::AwaitingPassword {
            token: Box::new(token),
        };
        step
    }

    /// Finish a login that arrived through raw TL, where grammers' private
    /// `complete_login` never ran — so the self-peer is cached here by hand.
    async fn complete_raw(
        &self,
        ctx: &DeviceLinkContext<'_>,
        link: &PendingLink,
        state: &mut PendingState,
        authorization: tl::enums::auth::Authorization,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let tl::enums::auth::Authorization::Authorization(authorization) = authorization else {
            // Third-party applications cannot register new accounts, so this
            // can never succeed however many times it is retried.
            return Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::AccountUnavailable,
                restartable: false,
            });
        };
        state.accepted = true;

        let info = self_peer(&authorization.user);
        link.connection
            .session()
            .cache_peer(&info)
            .await
            .map_err(custody_error)?;

        let (account_label, vendor_user_ref) = identity(&authorization.user);
        self.commit(ctx, link, state, account_label, vendor_user_ref)
            .await
    }

    /// Finish a login that came through a grammers high-level flow, which has
    /// already cached the self-peer for us.
    async fn commit(
        &self,
        ctx: &DeviceLinkContext<'_>,
        link: &PendingLink,
        state: &mut PendingState,
        account_label: String,
        vendor_user_ref: String,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        // Completion order is fixed: store the blob under compare-and-swap,
        // *then* report completion. Reporting first would tell the host a link
        // exists that no restart could recover.
        link.connection
            .session()
            .store_into(ctx.session)
            .await
            .map_err(custody_error)?;
        state.stored = true;
        // The link is dropped once custody is durable, freeing its runner and
        // sockets immediately. A poll *after* that answers
        // `Failed { restartable: true }` rather than `Completed` — the §4.3
        // miss semantics — which is correct: the host has already persisted the
        // terminal step, and a host that lost it should re-mint a link, which
        // `store_into`'s load-then-CAS lets it do over the blob just written.
        state.phase = PendingPhase::Completed {
            account_label: account_label.clone(),
            vendor_user_ref: vendor_user_ref.clone(),
        };
        self.pending.remove(ctx.flow_id);
        Ok(DeviceLinkStep::Completed {
            account_label,
            vendor_user_ref,
        })
    }

    async fn submit_identifier(
        &self,
        link: &PendingLink,
        state: &mut PendingState,
        phone: String,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        if !matches!(state.phase, PendingPhase::AwaitingIdentifier) {
            return Err(input_rejected(
                DeviceLinkInputKind::Identifier,
                "this link is not waiting for an account identifier",
            ));
        }
        // `request_login_code` is a grammers high-level flow that owns its own
        // datacenter-migration handling, so it goes around the transport
        // wrapper. Safe because the client is configured `NoRetries`: the call
        // fails once rather than silently re-sending.
        let token = link
            .connection
            .client()
            .request_login_code(&phone, self.api_hash.expose_secret())
            .await
            .map_err(invocation_error)?;
        state.phase = PendingPhase::AwaitingCode {
            token: Box::new(token),
        };
        Ok(code_prompt())
    }

    async fn submit_code(
        &self,
        ctx: &DeviceLinkContext<'_>,
        link: &PendingLink,
        state: &mut PendingState,
        code: SecretString,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        state.charge_attempt()?;
        let previous = std::mem::replace(&mut state.phase, PendingPhase::Failed);
        let PendingPhase::AwaitingCode { token } = previous else {
            state.phase = previous;
            return Err(input_rejected(
                DeviceLinkInputKind::Code,
                "this link is not waiting for a login code",
            ));
        };

        match link
            .connection
            .client()
            .sign_in(&token, code.expose_secret())
            .await
        {
            Ok(user) => {
                state.accepted = true;
                let (label, reference) = user_identity(&user);
                self.commit(ctx, link, state, label, reference).await
            }
            Err(SignInError::PasswordRequired(password)) => {
                state.accepted = false;
                Ok(self.ask_for_password(state, password))
            }
            Err(SignInError::InvalidCode) => {
                state.phase = PendingPhase::AwaitingCode { token };
                Err(input_rejected(
                    DeviceLinkInputKind::Code,
                    "that login code was not accepted",
                ))
            }
            Err(SignInError::SignUpRequired) => Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::AccountUnavailable,
                restartable: false,
            }),
            Err(SignInError::InvalidPassword(_)) => Err(input_rejected(
                DeviceLinkInputKind::Code,
                "that login code was not accepted",
            )),
            Err(SignInError::Other(error)) => {
                state.phase = PendingPhase::AwaitingCode { token };
                Err(invocation_error(error))
            }
        }
    }

    async fn submit_password(
        &self,
        ctx: &DeviceLinkContext<'_>,
        link: &PendingLink,
        state: &mut PendingState,
        password: SecretString,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        state.charge_attempt()?;
        let previous = std::mem::replace(&mut state.phase, PendingPhase::Failed);
        let PendingPhase::AwaitingPassword { token } = previous else {
            state.phase = previous;
            return Err(input_rejected(
                DeviceLinkInputKind::Password,
                "this link is not waiting for a password",
            ));
        };

        match link
            .connection
            .client()
            .check_password(*token, password.expose_secret().as_bytes())
            .await
        {
            Ok(user) => {
                state.accepted = true;
                let (label, reference) = user_identity(&user);
                self.commit(ctx, link, state, label, reference).await
            }
            // Telegram hands back a fresh token with fresh SRP parameters; the
            // old one is spent and cannot be reused for another attempt.
            Err(SignInError::InvalidPassword(retry)) => {
                state.phase = PendingPhase::AwaitingPassword {
                    token: Box::new(retry),
                };
                Err(input_rejected(
                    DeviceLinkInputKind::Password,
                    "that password was not accepted",
                ))
            }
            Err(SignInError::PasswordRequired(retry)) => Ok(self.ask_for_password(state, retry)),
            Err(SignInError::SignUpRequired) => Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::AccountUnavailable,
                restartable: false,
            }),
            Err(SignInError::InvalidCode) => Err(input_rejected(
                DeviceLinkInputKind::Password,
                "that password was not accepted",
            )),
            Err(SignInError::Other(error)) => Err(invocation_error(error)),
        }
    }
}

#[async_trait]
impl DeviceLinkAdapter for TelegramDeviceLinkAdapter {
    async fn begin(
        &self,
        ctx: &DeviceLinkContext<'_>,
        mode: DeviceLinkMode,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.reap_expired().await;
        match mode {
            DeviceLinkMode::Default => self.begin_scan(ctx).await,
            DeviceLinkMode::Alternate => self.begin_identifier(ctx).await,
        }
    }

    async fn poll(&self, ctx: &DeviceLinkContext<'_>) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.reap_expired().await;
        let Some(link) = self.pending.get(ctx.flow_id) else {
            // A restarted process or a reaped flow. Restartable, so the card
            // mints a fresh link through the existing step path (§4.3).
            return Ok(DeviceLinkStep::Failed {
                code: DeviceLinkErrorCode::UnknownFlow,
                restartable: true,
            });
        };
        let mut state = link.state.lock().await;
        match &state.phase {
            // Only the scan phase advances on a poll. While the flow waits on
            // the user, `poll` is a pure read — the host polls it concurrently
            // with whatever is being typed.
            PendingPhase::AwaitingScan { .. } => self.drive_scan(ctx, &link, &mut state).await,
            PendingPhase::AwaitingIdentifier => Ok(identifier_prompt()),
            PendingPhase::AwaitingCode { .. } => Ok(code_prompt()),
            PendingPhase::AwaitingPassword { token } => Ok(password_prompt(token)),
            PendingPhase::Completed {
                account_label,
                vendor_user_ref,
            } => Ok(DeviceLinkStep::Completed {
                account_label: account_label.clone(),
                vendor_user_ref: vendor_user_ref.clone(),
            }),
            PendingPhase::Failed => Ok(DeviceLinkStep::Failed {
                code: DeviceLinkErrorCode::Internal,
                restartable: true,
            }),
        }
    }

    async fn submit_input(
        &self,
        ctx: &DeviceLinkContext<'_>,
        input: DeviceLinkInput,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        input.validate()?;
        self.reap_expired().await;
        let Some(link) = self.pending.get(ctx.flow_id) else {
            return Err(DeviceLinkError::UnknownFlow);
        };
        let mut state = link.state.lock().await;
        match input {
            DeviceLinkInput::Identifier(phone) => {
                self.submit_identifier(&link, &mut state, phone).await
            }
            DeviceLinkInput::Code(code) => self.submit_code(ctx, &link, &mut state, code).await,
            DeviceLinkInput::Password(password) => {
                self.submit_password(ctx, &link, &mut state, password).await
            }
        }
    }

    async fn cancel(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError> {
        if let Some(link) = self.pending.remove(ctx.flow_id) {
            abandon(link).await;
        }
        Ok(())
    }

    async fn revoke(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError> {
        let Some(grant) = ctx.account else {
            return Err(DeviceLinkError::Internal {
                reason: "revoking a linked account requires a resolved credential account",
            });
        };
        // Both outcomes return `Ok`: local deletion proceeds either way
        // (PROPOSAL §4.5), and an `Err` here would tell the host the teardown
        // failed, stranding the durable state. The contract has no
        // "explicitly unverified" variant, so the distinction is only
        // observable in the log — recorded as a known gap.
        match self.revoker.revoke(grant).await {
            RevokeOutcome::LoggedOut => debug!("telegram device authorization ended"),
            RevokeOutcome::LogoutUnverified => {
                debug!("telegram device authorization could not be confirmed as ended");
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Parked links
// ---------------------------------------------------------------------------

/// The bounded, TTL'd registry of in-progress links.
///
/// A [`std::sync::Mutex`], so it cannot be held across an `await` even by
/// accident; every async abort path takes the `Arc` out first and works on it
/// outside the lock.
#[derive(Default)]
struct PendingLinks {
    entries: Mutex<HashMap<DeviceLinkFlowId, Arc<PendingLink>>>,
}

impl PendingLinks {
    /// Whether one more link would fit. Replacing an existing flow always
    /// fits: it consumes no additional slot.
    fn check_capacity(&self, flow_id: &DeviceLinkFlowId) -> Result<(), DeviceLinkError> {
        let entries = self.lock()?;
        if entries.len() >= MAX_PENDING_LINKS && !entries.contains_key(flow_id) {
            // Host-side capacity, not a vendor limit. `RateLimited` is the only
            // code that means "this can work, shortly" — which is the truth.
            return Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::RateLimited,
                restartable: true,
            });
        }
        Ok(())
    }

    fn insert(
        &self,
        flow_id: DeviceLinkFlowId,
        link: Arc<PendingLink>,
    ) -> Result<(), DeviceLinkError> {
        self.check_capacity(&flow_id)?;
        self.lock()?.insert(flow_id, link);
        Ok(())
    }

    fn get(&self, flow_id: &DeviceLinkFlowId) -> Option<Arc<PendingLink>> {
        self.entries.lock().ok()?.get(flow_id).map(Arc::clone)
    }

    fn remove(&self, flow_id: &DeviceLinkFlowId) -> Option<Arc<PendingLink>> {
        self.entries.lock().ok()?.remove(flow_id)
    }

    fn take_expired(&self) -> Vec<Arc<PendingLink>> {
        let Ok(mut entries) = self.entries.lock() else {
            return Vec::new();
        };
        let expired = entries
            .iter()
            .filter(|(_, link)| link.created_at.elapsed() >= PENDING_LINK_TTL)
            .map(|(flow_id, _)| flow_id.clone())
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|flow_id| entries.remove(&flow_id))
            .collect()
    }

    fn drain(&self) -> Vec<Arc<PendingLink>> {
        let Ok(mut entries) = self.entries.lock() else {
            return Vec::new();
        };
        entries.drain().map(|(_, link)| link).collect()
    }

    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, HashMap<DeviceLinkFlowId, Arc<PendingLink>>>,
        DeviceLinkError,
    > {
        self.entries.lock().map_err(|_| DeviceLinkError::Internal {
            reason: "the pending device-link registry lock was poisoned",
        })
    }
}

/// One parked login: its connection, and the mutex that serializes every vendor
/// call against it.
struct PendingLink {
    connection: MtprotoConnection,
    state: tokio::sync::Mutex<PendingState>,
    created_at: Instant,
}

struct PendingState {
    phase: PendingPhase,
    /// Telegram has issued an authorization for this device.
    accepted: bool,
    /// Custody holds the resulting session, so abandoning the link must **not**
    /// log out — that would destroy the credential just stored.
    stored: bool,
    attempts: u8,
    export_backoff: Duration,
    /// `serverNow - localNow`, in seconds. Token expiry is server time.
    server_offset: i64,
}

impl Default for PendingState {
    fn default() -> Self {
        Self {
            phase: PendingPhase::AwaitingIdentifier,
            accepted: false,
            stored: false,
            attempts: 0,
            export_backoff: INITIAL_EXPORT_BACKOFF,
            server_offset: 0,
        }
    }
}

impl PendingState {
    fn charge_attempt(&mut self) -> Result<(), DeviceLinkError> {
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts > MAX_INPUT_ATTEMPTS {
            return Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::RateLimited,
                restartable: true,
            });
        }
        Ok(())
    }
}

enum PendingPhase {
    AwaitingScan {
        token: Vec<u8>,
    },
    AwaitingIdentifier,
    AwaitingCode {
        token: Box<LoginToken>,
    },
    AwaitingPassword {
        token: Box<PasswordToken>,
    },
    Completed {
        account_label: String,
        vendor_user_ref: String,
    },
    Failed,
}

/// End a parked link, logging out first when Telegram authorized a device this
/// process never made durable.
async fn abandon(link: Arc<PendingLink>) {
    let needs_logout = {
        let state = link.state.lock().await;
        state.accepted && !state.stored
    };
    if !needs_logout {
        return;
    }
    let call = link
        .connection
        .invoke(&tl::functions::auth::LogOut {}, VendorOpKind::Write);
    match tokio::time::timeout(LOGOUT_TIMEOUT, call).await {
        Ok(Ok(_)) => {}
        // Both remaining arms leave a device Telegram may still consider
        // authorized. Nothing here can fix that — the user can, from
        // Telegram's own device list — so it is recorded rather than
        // swallowed, and the product copy tells them to look.
        Ok(Err(_)) => debug!("logging out an abandoned telegram device link failed"),
        Err(_) => debug!("logging out an abandoned telegram device link timed out"),
    }
}

// ---------------------------------------------------------------------------
// Vendor helpers
// ---------------------------------------------------------------------------

/// Read Telegram's clock so token expiry can be judged in *server* time, as
/// TDLib, Web A and Web K all do. `help.getConfig` carries `date`, which is the
/// only server timestamp this flow can reach.
async fn server_offset(connection: &MtprotoConnection) -> i64 {
    match connection
        .invoke(&tl::functions::help::GetConfig {}, VendorOpKind::Read)
        .await
    {
        Ok(tl::enums::Config::Config(config)) => i64::from(config.date) - local_unix_seconds(),
        Err(_) => 0,
    }
}

/// Re-read the 2FA parameters. They must be **fresh**: `PasswordToken` carries
/// SRP values that are single-use, so a cached one cannot answer a second
/// prompt.
async fn fetch_password_token(link: &PendingLink) -> Result<PasswordToken, DeviceLinkError> {
    let password: tl::enums::account::Password = link
        .connection
        .invoke(&tl::functions::account::GetPassword {}, VendorOpKind::Read)
        .await
        .map_err(vendor_error)?;
    Ok(PasswordToken::new(password.into()))
}

/// The self-peer, cached by hand because raw TL bypassed grammers' private
/// `complete_login`. Without it, the very first `resolve_peer` for the user's
/// own account misses.
fn self_peer(user: &tl::enums::User) -> PeerInfo {
    match PeerInfo::from(user) {
        PeerInfo::User { id, auth, bot, .. } => PeerInfo::User {
            id,
            auth,
            bot,
            is_self: Some(true),
        },
        other => other,
    }
}

/// What the card shows after completion: a human label, and the vendor's own
/// stable identifier so a substituted login is *observable* rather than silent.
fn identity(user: &tl::enums::User) -> (String, String) {
    let tl::enums::User::User(user) = user else {
        return ("Telegram account".to_string(), "unknown".to_string());
    };
    let reference = user.id.to_string();
    let label = user
        .username
        .as_deref()
        .map(|username| format!("@{username}"))
        .or_else(|| {
            let full = [user.first_name.as_deref(), user.last_name.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            (!full.trim().is_empty()).then_some(full)
        })
        .unwrap_or_else(|| reference.clone());
    (label, reference)
}

/// The same projection for the high-level phone path, which hands back a
/// `User` rather than raw TL.
fn user_identity(user: &grammers_client::peer::User) -> (String, String) {
    let reference = user.id().bare_id_unchecked().to_string();
    let label = user
        .username()
        .map(|username| format!("@{username}"))
        .or_else(|| {
            let full = user.full_name();
            (!full.trim().is_empty()).then_some(full)
        })
        .unwrap_or_else(|| reference.clone());
    (label, reference)
}

/// `tg://login?token=<base64url>` — the payload an official client scans.
fn login_payload(token: &[u8]) -> Result<DeviceLinkPayload, DeviceLinkStep> {
    DeviceLinkPayload::new(format!("tg://login?token={}", BASE64_URL.encode(token))).map_err(|_| {
        DeviceLinkStep::Failed {
            code: DeviceLinkErrorCode::Internal,
            restartable: true,
        }
    })
}

/// Seconds left on a token, judged against the server's clock.
fn remaining_for(expires: i32, server_offset: i64) -> Duration {
    let server_now = local_unix_seconds() + server_offset;
    let remaining = i64::from(expires) - server_now;
    Duration::from_secs(remaining.max(0).unsigned_abs())
}

/// `min(3s, expires − serverNow)`, floored so an about-to-expire token cannot
/// spin the poller.
fn poll_interval(remaining: Duration) -> Duration {
    remaining.min(MAX_POLL_INTERVAL).max(MIN_POLL_INTERVAL)
}

fn local_unix_seconds() -> i64 {
    // silent-ok: a clock before the epoch yields a zero offset, which degrades
    // to local time — the same behaviour as having no server reading at all.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

/// The three user-facing prompts, each written once. `poll` and the submit
/// path must ask the same question in the same words, or a card that polls
/// mid-typing visibly rewrites itself.
fn identifier_prompt() -> DeviceLinkStep {
    prompt(
        DeviceLinkInputKind::Identifier,
        "Phone number",
        Some("Include the country code, for example +1 415 555 0132."),
    )
}

fn code_prompt() -> DeviceLinkStep {
    prompt(
        DeviceLinkInputKind::Code,
        "Login code",
        Some("Telegram sent a code to your other signed-in devices."),
    )
}

/// The vendor's own password hint, when it set one. An all-whitespace hint is
/// dropped rather than passed on: `DeviceLinkStep::validate` refuses empty
/// display text, so forwarding it would turn a 2FA prompt into a rejected step.
fn password_prompt(token: &PasswordToken) -> DeviceLinkStep {
    let hint = token.hint().map(str::trim).filter(|hint| !hint.is_empty());
    prompt(
        DeviceLinkInputKind::Password,
        "Two-step verification password",
        hint,
    )
}

fn prompt(kind: DeviceLinkInputKind, label: &str, hint: Option<&str>) -> DeviceLinkStep {
    DeviceLinkStep::InputRequired {
        kind,
        label: label.to_string(),
        hint: hint.map(str::to_string),
    }
}

fn input_rejected(kind: DeviceLinkInputKind, reason: &'static str) -> DeviceLinkError {
    DeviceLinkError::InvalidInput { kind, reason }
}

#[cfg(test)]
#[path = "../tests/linked_login.rs"]
mod tests;
