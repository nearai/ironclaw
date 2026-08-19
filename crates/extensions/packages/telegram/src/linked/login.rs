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
//! the host's device-link cadence (`DEVICE_LINK_POLL_INTERVAL_MILLIS`, 3 s),
//! repainting the live code on **every** poll, and correct for **server** time.
//! That is what keeps `drop(updates)` a global rule (PROPOSAL §4.2).
//!
//! Repainting unconditionally is deliberate. An earlier revision answered
//! `AwaitingVendor` when the re-exported bytes were unchanged — which is what
//! they are for the whole of a token's window — and since that variant means
//! "nothing to show", the card blanked a still-valid QR one poll after painting
//! it. Identical bytes repaint identically, so there is nothing to churn.
//!
//! # Logout on abort, never in `Drop`
//!
//! Once Telegram has authorized the device, walking away leaves a live
//! authorization the host has forgotten. Every abort path — TTL reap, cancel,
//! shutdown — therefore *awaits* [`abandon`], which calls `auth.logOut`. It
//! cannot live in `Drop`: `Drop` is synchronous, `logOut` is not, and the same
//! value aborts its runner on drop, so a `tokio::spawn` there would be a race
//! with shutdown that silently does nothing.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL;
use grammers_client::InvocationError;
use grammers_client::client::{PasswordToken, SignInError};
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
use crate::linked::transport::{MtprotoConnection, TransportError, VendorOpKind};

mod errors;
mod pending;

use errors::{custody_error, fatal_step, invocation_error, login_requires_password, vendor_error};
use pending::{PendingLink, PendingLinks, PendingPhase, PendingState, abandon};

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
/// The operator's MTProto application identity, from `[admin_configuration]`
/// (`telegram_api_id` / `telegram_api_hash`) — the *developer application's*
/// credentials, not the user's. `api_hash` is declared `secret = true` because
/// Telegram treats it as one.
pub struct MtprotoAppIdentity {
    pub api_id: i32,
    pub api_hash: SecretString,
}

/// The vendor half of Telegram's device link.
pub struct TelegramDeviceLinkAdapter {
    /// `None` when the deployment has not configured its MTProto identity —
    /// both admin fields are `required = false` on purpose, so a bot-only
    /// deployment keeps activating. Every link attempt then fails closed with
    /// an explicit not-configured error instead of dialing anything.
    identity: Option<MtprotoAppIdentity>,
    revoker: LinkedAccountRevoker,
    pending: PendingLinks,
}

impl TelegramDeviceLinkAdapter {
    /// Build the adapter.
    ///
    /// The identity arrives at construction rather than through
    /// [`DeviceLinkContext::config`], which carries non-secret values only —
    /// the binary resolves `telegram_api_hash` at load time (the one I/O-legal
    /// point) and an admin-configuration edit re-runs load via reactivation.
    ///
    /// The pool is borrowed to mint a **narrow** revoke handle; the adapter
    /// never holds the pool itself. It runs inside an auth flow with no
    /// capability authorization, no approval, and no origin gate, so a handle
    /// to every user's live authenticated client is exactly what it must not
    /// have (PROPOSAL §3.3).
    pub fn new(identity: Option<MtprotoAppIdentity>, pool: &SessionPool) -> Self {
        Self {
            identity,
            revoker: pool.revoker(),
            pending: PendingLinks::default(),
        }
    }

    /// The configured application identity, or the explicit not-configured
    /// failure the manifest promises ("fails closed with an explicit error
    /// rather than making every existing install invalid").
    fn identity(&self) -> Result<&MtprotoAppIdentity, DeviceLinkError> {
        self.identity.as_ref().ok_or(DeviceLinkError::Internal {
            reason: "the deployment has not configured its MTProto application identity \
                     (telegram_api_id / telegram_api_hash)",
        })
    }

    /// Abort every parked link, logging out any that Telegram already
    /// authorized. Called on shutdown, within a bounded grace period.
    pub async fn shutdown(&self) {
        for link in self.pending.drain() {
            abandon(link).await;
        }
    }

    /// Reap links that outlived [`crate::linked::PENDING_LINK_TTL`], logging
    /// each out first.
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
        let identity = self.identity()?;
        self.pending.check_capacity(flow_id)?;
        let session = IronclawSession::in_memory();
        let link = Arc::new(PendingLink::new(MtprotoConnection::open(
            session,
            identity.api_id,
        )));
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
        let identity = self.identity()?;
        let request = tl::functions::auth::ExportLoginToken {
            api_id: identity.api_id,
            api_hash: identity.api_hash.expose_secret().to_string(),
            // Identical on every poll, by contract: a changed set mints a new
            // token and invalidates a scan already in progress.
            except_ids: Vec::new(),
        };
        match link.connection.invoke(&request, VendorOpKind::Read).await {
            Ok(token) => self.apply_login_token(ctx, link, state, token).await,
            // For a same-datacenter account, 2FA surfaces on the export call
            // itself. (An account on another datacenter surfaces it on the
            // `ImportLoginToken` hop instead — handled in
            // `apply_login_token`'s `MigrateTo` arm.)
            Err(ref error) if login_requires_password(error) => {
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
                    return Ok(paint_token(state, exported));
                }
                tl::enums::auth::LoginToken::Success(success) => {
                    return self
                        .complete_raw(ctx, link, state, success.authorization)
                        .await;
                }
                tl::enums::auth::LoginToken::MigrateTo(migrate) => {
                    let imported = match link
                        .connection
                        .invoke_in_dc(
                            migrate.dc_id,
                            &tl::functions::auth::ImportLoginToken {
                                token: migrate.token,
                            },
                            VendorOpKind::Read,
                        )
                        .await
                    {
                        Ok(imported) => imported,
                        // The scan was accepted on the migrated datacenter and
                        // the account's second factor now gates the session:
                        // this is the 2FA branch, not a failure (live repro:
                        // QA 2026-08-14T14:49Z). Persist the datacenter move
                        // first — the SRP exchange must run where the login is
                        // pending, exactly as the success arm does before
                        // `complete_raw`.
                        Err(ref error) if login_requires_password(error) => {
                            link.connection
                                .session()
                                .set_home_dc_id(migrate.dc_id)
                                .await
                                .map_err(custody_error)?;
                            let token = fetch_password_token(link).await?;
                            return Ok(self.ask_for_password(state, token));
                        }
                        Err(error) => return Err(vendor_error(error)),
                    };
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
        self.complete_user(ctx, link, state, authorization.user)
            .await
    }

    /// Finish from a raw user after Telegram has authorized this session.
    ///
    /// The phone-code path normally lets grammers perform this bookkeeping.
    /// If grammers reports a later bookkeeping failure after Telegram already
    /// accepted the code, probing the session and landing here is what keeps a
    /// live device from being misreported as an unavailable account.
    async fn complete_user(
        &self,
        ctx: &DeviceLinkContext<'_>,
        link: &PendingLink,
        state: &mut PendingState,
        user: tl::enums::User,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        state.accepted = true;
        let info = self_peer(&user);
        link.connection
            .session()
            .cache_peer(&info)
            .await
            .map_err(custody_error)?;

        let (account_label, vendor_user_ref) = identity(&user);
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
        // Keep the authorized connection parked until the host acknowledges
        // that credential custody and product identity both committed. A
        // failure after this return is still provisional and `cancel` must be
        // able to log it out rather than strand a Telegram session.
        state.phase = PendingPhase::Completed {
            account_label: account_label.clone(),
            vendor_user_ref: vendor_user_ref.clone(),
        };
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
            .request_login_code(&phone, self.identity()?.api_hash.expose_secret())
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
            Err(SignInError::SignUpRequired) => {
                debug!(
                    home_dc = link.connection.session().home_dc_id().ok(),
                    "telegram sign-in reported sign-up-required; reconciling authorization"
                );
                match resolve_post_sign_in_failure(PostSignInFailure::SignUpRequired, || {
                    recover_authorized_user(link)
                })
                .await
                {
                    Ok(PostSignInResolution::Authorized(user)) => {
                        debug!(
                            "telegram sign-in returned sign-up-required after authorization; recovering the live session"
                        );
                        self.complete_user(ctx, link, state, *user).await
                    }
                    Ok(PostSignInResolution::Unregistered) => Err(DeviceLinkError::Vendor {
                        code: DeviceLinkErrorCode::AccountUnavailable,
                        restartable: false,
                    }),
                    Ok(PostSignInResolution::Original(_)) => Err(DeviceLinkError::Internal {
                        reason: "sign-up-required recovery produced an impossible original failure",
                    }),
                    Err(error) => {
                        // The sign-in result and the authorization probe now
                        // disagree. Treat the device as possibly accepted so
                        // the flow's teardown attempts auth.logOut rather than
                        // leaving an authorization the host cannot address.
                        state.accepted = true;
                        Err(error)
                    }
                }
            }
            Err(SignInError::InvalidPassword(_)) => Err(input_rejected(
                DeviceLinkInputKind::Code,
                "that login code was not accepted",
            )),
            Err(SignInError::Other(error)) => {
                match resolve_post_sign_in_failure(PostSignInFailure::Original(error), || {
                    recover_authorized_user(link)
                })
                .await
                {
                    Ok(PostSignInResolution::Authorized(user)) => {
                        debug!(
                            "telegram sign-in bookkeeping failed after authorization; recovering the live session"
                        );
                        self.complete_user(ctx, link, state, *user).await
                    }
                    Ok(PostSignInResolution::Original(error)) => {
                        state.phase = PendingPhase::AwaitingCode { token };
                        Err(invocation_error(error))
                    }
                    Ok(PostSignInResolution::Unregistered) => Err(DeviceLinkError::Internal {
                        reason: "an original sign-in failure became an unregistered account",
                    }),
                    Err(error) => {
                        state.accepted = true;
                        Err(error)
                    }
                }
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
            Err(SignInError::SignUpRequired) => {
                match resolve_post_sign_in_failure(PostSignInFailure::SignUpRequired, || {
                    recover_authorized_user(link)
                })
                .await
                {
                    Ok(PostSignInResolution::Authorized(user)) => {
                        self.complete_user(ctx, link, state, *user).await
                    }
                    Ok(PostSignInResolution::Unregistered) => Err(DeviceLinkError::Vendor {
                        code: DeviceLinkErrorCode::AccountUnavailable,
                        restartable: false,
                    }),
                    Ok(PostSignInResolution::Original(_)) => Err(DeviceLinkError::Internal {
                        reason: "sign-up-required recovery produced an impossible original failure",
                    }),
                    Err(error) => {
                        state.accepted = true;
                        Err(error)
                    }
                }
            }
            Err(SignInError::InvalidCode) => Err(input_rejected(
                DeviceLinkInputKind::Password,
                "that password was not accepted",
            )),
            Err(SignInError::Other(error)) => {
                match resolve_post_sign_in_failure(PostSignInFailure::Original(error), || {
                    recover_authorized_user(link)
                })
                .await
                {
                    Ok(PostSignInResolution::Authorized(user)) => {
                        self.complete_user(ctx, link, state, *user).await
                    }
                    Ok(PostSignInResolution::Original(error)) => Err(invocation_error(error)),
                    Ok(PostSignInResolution::Unregistered) => Err(DeviceLinkError::Internal {
                        reason: "an original password failure became an unregistered account",
                    }),
                    Err(error) => {
                        state.accepted = true;
                        Err(error)
                    }
                }
            }
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
            PendingPhase::AwaitingScan => self.drive_scan(ctx, &link, &mut state).await,
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

    async fn finalize(&self, ctx: &DeviceLinkContext<'_>) {
        // Host-side custody and identity are now durable. Dropping the parked
        // connection without `abandon` preserves the established Telegram
        // authorization while releasing the provisional rollback handle.
        self.pending.remove(ctx.flow_id);
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

/// A failure from grammers' high-level sign-in call before it is reconciled
/// with the session's actual authorization state.
enum PostSignInFailure {
    SignUpRequired,
    Original(InvocationError),
}

/// The reconciled answer. Telegram's session is authoritative: a replayed
/// `SignUpRequired` after the code already landed must not override a live
/// authorization.
enum PostSignInResolution {
    Authorized(Box<tl::enums::User>),
    Unregistered,
    Original(InvocationError),
}

async fn resolve_post_sign_in_failure<F, Fut>(
    failure: PostSignInFailure,
    probe: F,
) -> Result<PostSignInResolution, DeviceLinkError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<tl::enums::User>, DeviceLinkError>>,
{
    if let Some(user) = probe().await? {
        return Ok(PostSignInResolution::Authorized(Box::new(user)));
    }
    Ok(match failure {
        PostSignInFailure::SignUpRequired => PostSignInResolution::Unregistered,
        PostSignInFailure::Original(error) => PostSignInResolution::Original(error),
    })
}

/// Read back the session after an apparently failed sign-in.
///
/// Telegram can accept the code before grammers finishes its local peer-cache
/// bookkeeping. The authorization read and `UserSelf` lookup turn that
/// partially reported success into a durable completion instead of asking the
/// user to consume the same one-time code again.
async fn recover_authorized_user(
    link: &PendingLink,
) -> Result<Option<tl::enums::User>, DeviceLinkError> {
    match link
        .connection
        .invoke(&tl::functions::updates::GetState {}, VendorOpKind::Read)
        .await
    {
        Ok(_) => {}
        Err(TransportError::Rpc {
            code: 401, name, ..
        }) => {
            debug!(
                rpc_name = %name,
                home_dc = link.connection.session().home_dc_id().ok(),
                "telegram post-sign-in authorization probe was rejected"
            );
            return Ok(None);
        }
        Err(error) => return Err(vendor_error(error)),
    }

    let users = link
        .connection
        .invoke(
            &tl::functions::users::GetUsers {
                id: vec![tl::enums::InputUser::UserSelf],
            },
            VendorOpKind::Read,
        )
        .await
        .map_err(vendor_error)?;
    users
        .into_iter()
        .find(|user| matches!(user, tl::enums::User::User(_)))
        .map(Some)
        .ok_or(DeviceLinkError::Internal {
            reason: "telegram authorized the session but returned no current user",
        })
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
/// Record a freshly exported token and paint the code it carries.
///
/// **Always a `Display`, unchanged bytes included.** Within the token's
/// window the server returns the same bytes on every poll, and this used
/// to answer `AwaitingVendor` for that case. That variant means "nothing
/// to show" (`DeviceLinkStep::AwaitingVendor`), so the card blanked the QR
/// roughly one poll after painting it and sat on "waiting for the vendor"
/// while the still-valid code was never displayed again — a link nobody
/// could complete.
///
/// Re-emitting the same payload is not the churn the old comment feared:
/// the code a user is mid-scan on only changes when the *bytes* change, and
/// identical bytes repaint identically. `expires_in` is recomputed each
/// time, so the countdown stays honest. It costs no extra durable write
/// either — the driver applies whatever step comes back, so both arms
/// already wrote a revision.
fn paint_token(state: &mut PendingState, exported: tl::types::auth::LoginToken) -> DeviceLinkStep {
    let remaining = remaining_for(exported.expires, state.server_offset);
    state.export_backoff = INITIAL_EXPORT_BACKOFF;
    state.phase = PendingPhase::AwaitingScan;

    match login_payload(&exported.token) {
        Ok(payload) => DeviceLinkStep::Display {
            kind: DeviceLinkDisplayKind::QrCode,
            payload,
            expires_in: remaining,
        },
        Err(step) => step,
    }
}

fn remaining_for(expires: i32, server_offset: i64) -> Duration {
    let server_now = local_unix_seconds() + server_offset;
    let remaining = i64::from(expires) - server_now;
    Duration::from_secs(remaining.max(0).unsigned_abs())
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
        Some(
            "Telegram chooses where this code goes: usually a message from \"Telegram\" (the \
             service chat) in your other signed-in Telegram apps, sometimes an SMS. Check \
             both; if nothing arrives within a minute, choose Start again to request a new \
             code, or link by scanning the code instead.",
        ),
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
