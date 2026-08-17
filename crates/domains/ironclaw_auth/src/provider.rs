use std::fmt;

use async_trait::async_trait;
use ironclaw_extension_contracts::device_link::{
    DeviceLinkErrorCode, DeviceLinkInput, DeviceLinkMode, DeviceLinkStep,
    MAX_DEVICE_LINK_LABEL_BYTES,
};
use ironclaw_host_api::error::HostApiError;
use ironclaw_host_api::ids::{ExtensionId, SecretHandle, UserId};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    AuthFlowId, AuthProductError, AuthProductScope, AuthorizationCodeHash, CredentialAccountId,
    CredentialAccountLabel, OAuthProviderIdentity, PkceVerifierHash, ProviderScope,
    ids::AuthProviderId,
};

macro_rules! one_shot_secret {
    ($name:ident, $label:literal) => {
        pub struct $name(SecretString);

        impl $name {
            pub fn new(value: SecretString) -> Result<Self, AuthProductError> {
                let exposed = value.expose_secret();
                if exposed.is_empty() {
                    return Err(AuthProductError::invalid_request(format!(
                        "{} must not be empty",
                        $label
                    )));
                }
                if exposed.trim() != exposed {
                    return Err(AuthProductError::invalid_request(format!(
                        "{} must not contain leading or trailing whitespace",
                        $label
                    )));
                }
                if exposed.chars().any(|c| c == '\0' || c.is_control()) {
                    return Err(AuthProductError::invalid_request(format!(
                        "{} must not contain NUL/control characters",
                        $label
                    )));
                }
                Ok(Self(value))
            }

            pub fn expose_secret(&self) -> &str {
                self.0.expose_secret()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    };
}

one_shot_secret!(OAuthAuthorizationCode, "oauth authorization code");
one_shot_secret!(PkceVerifierSecret, "pkce verifier");

/// One-shot provider exchange input. This type intentionally does not implement
/// serde traits because it may carry raw OAuth code and PKCE verifier material.
pub struct OAuthProviderCallbackRequest {
    pub provider: AuthProviderId,
    pub account_label: CredentialAccountLabel,
    pub authorization_code: OAuthAuthorizationCode,
    pub authorization_code_hash: AuthorizationCodeHash,
    pub pkce_verifier: PkceVerifierSecret,
    pub pkce_verifier_hash: PkceVerifierHash,
    pub scopes: Vec<ProviderScope>,
}

impl fmt::Debug for OAuthProviderCallbackRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthProviderCallbackRequest")
            .field("provider", &self.provider)
            .field("account_label", &self.account_label)
            .field("authorization_code", &"[REDACTED]")
            .field("authorization_code_hash", &self.authorization_code_hash)
            .field("pkce_verifier", &"[REDACTED]")
            .field("pkce_verifier_hash", &self.pkce_verifier_hash)
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// Provider-exchange context claimed by the product-auth flow before raw
/// provider material is exchanged or stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderExchangeContext {
    pub scope: AuthProductScope,
    pub flow_id: AuthFlowId,
}

/// Provider-exchange result safe to store in auth-flow/account records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderExchange {
    pub provider: AuthProviderId,
    pub account_label: CredentialAccountLabel,
    pub authorization_code_hash: AuthorizationCodeHash,
    pub pkce_verifier_hash: PkceVerifierHash,
    pub access_secret: SecretHandle,
    pub refresh_secret: Option<SecretHandle>,
    pub scopes: Vec<ProviderScope>,
    pub account_id: Option<CredentialAccountId>,
    pub provider_identity: Option<OAuthProviderIdentity>,
}

/// One-shot provider refresh input. This type intentionally does not implement
/// serde traits because refresh authority must stay behind host-mediated
/// credential/egress boundaries.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthProviderRefreshRequest {
    pub provider: AuthProviderId,
    pub scope: AuthProductScope,
    pub account_id: CredentialAccountId,
    pub refresh_secret: SecretHandle,
    pub scopes: Vec<ProviderScope>,
}

impl fmt::Debug for OAuthProviderRefreshRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthProviderRefreshRequest")
            .field("provider", &self.provider)
            .field("scope", &self.scope)
            .field("account_id", &self.account_id)
            .field("refresh_secret", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// Provider refresh result safe to store back into credential-account records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderRefresh {
    pub provider: AuthProviderId,
    pub access_secret: SecretHandle,
    pub refresh_secret: Option<SecretHandle>,
    pub scopes: Vec<ProviderScope>,
}

#[async_trait]
pub trait AuthProviderClient: Send + Sync {
    async fn exchange_callback(
        &self,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError>;

    async fn exchange_callback_for_requester(
        &self,
        _requester_extension: Option<ExtensionId>,
        context: OAuthProviderExchangeContext,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        self.exchange_callback(context, request).await
    }

    async fn refresh_token(
        &self,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError>;

    /// Auth orchestration may retain requester identity separately from the
    /// provider-facing refresh DTO. Implementations that do not resolve
    /// recipes can use the safe default.
    async fn refresh_token_for_requester(
        &self,
        _requester_extension: Option<ExtensionId>,
        request: OAuthProviderRefreshRequest,
    ) -> Result<OAuthProviderRefresh, AuthProductError> {
        self.refresh_token(request).await
    }

    async fn cleanup_exchange(
        &self,
        _context: OAuthProviderExchangeContext,
        _exchange: &OAuthProviderExchange,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

/// The vendor's own identifier for the account a link resolved to, shown back
/// to the user after completion.
///
/// **This is a security control, not decoration.** The host cannot verify that
/// a device-link adapter displayed a payload for *this* user's account — it
/// does not speak the vendor protocol. Rendering the resolved identity is what
/// makes a substituted login observable (PROPOSAL §3.2; the ADR records that
/// this is detection, not prevention), so the value is bounded and validated
/// here rather than trusted as free text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceLinkVendorUserRef(String);

impl DeviceLinkVendorUserRef {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "device_link_vendor_user_ref",
            value: value.clone(),
            reason: reason.to_string(),
        };
        if value.is_empty() {
            return Err(invalid("must not be empty"));
        }
        if value.len() > MAX_DEVICE_LINK_LABEL_BYTES {
            return Err(invalid("exceeds the maximum vendor user-ref length"));
        }
        if value.chars().any(char::is_control) {
            return Err(invalid("must not contain control characters"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceLinkVendorUserRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Who a device-link call acts for.
///
/// `scope` is the durable flow's own product scope — not anything a card or
/// an adapter supplied. It rides the binding because the driver
/// implementation must mint the credential account at completion, and
/// synthesizing an [`AuthProductScope`] from a bare user id would be
/// re-deriving security-relevant scope, which this repo bans. The flow's
/// owner is `scope.resource.user_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkBinding {
    pub provider: AuthProviderId,
    pub extension_id: ExtensionId,
    pub scope: AuthProductScope,
}

impl DeviceLinkBinding {
    /// The flow's owner.
    pub fn user_id(&self) -> &UserId {
        &self.scope.resource.user_id
    }
}

/// Start a link on the chosen path.
#[derive(Debug)]
pub struct DeviceLinkBeginRequest {
    pub flow_id: AuthFlowId,
    pub binding: DeviceLinkBinding,
    pub mode: DeviceLinkMode,
}

/// Ask the vendor whether an in-flight link has moved.
///
/// "Pure read" in the sense that matters: safe to call at the host's cadence
/// without consuming a one-shot or advancing anything the user has not done.
/// It may still talk to the vendor — a handshake whose acceptance is only
/// observable by re-asking has no other way to notice, and the shipped
/// implementation works exactly that way — so this is not a no-side-effects
/// claim, and an implementor reading it as one would ship a poll that can
/// never complete.
#[derive(Debug)]
pub struct DeviceLinkPollRequest {
    pub flow_id: AuthFlowId,
    pub binding: DeviceLinkBinding,
}

/// Hand the vendor one value the previous step asked for.
///
/// Deliberately not `Clone` and not `Serialize`: [`DeviceLinkInput`] carries a
/// login code or an account password, so a request that could be copied into a
/// record or a log line would defeat the whole containment.
#[derive(Debug)]
pub struct DeviceLinkSubmitRequest {
    pub flow_id: AuthFlowId,
    pub binding: DeviceLinkBinding,
    pub input: DeviceLinkInput,
}

/// Why a link is being abandoned. The driver logs out vendor-side on every
/// post-acceptance abort, so it needs to know which abort this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceLinkCancelReason {
    /// The user pressed cancel.
    UserCanceled,
    /// The flow clock lapsed.
    FlowExpired,
    /// The transition the vendor reported cannot be carried further.
    Terminal,
}

/// Abandon an in-progress link.
#[derive(Debug)]
pub struct DeviceLinkCancelRequest {
    pub flow_id: AuthFlowId,
    pub binding: DeviceLinkBinding,
    pub reason: DeviceLinkCancelReason,
}

/// The credential account a completed link resolved to.
///
/// Its presence is what lets auth report `Completed`: custody is durable and
/// the account is minted *before* the driver returns this, so the flow never
/// announces a link the system cannot serve (PROPOSAL §4.3, completion order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkLinkedAccount {
    pub account_id: CredentialAccountId,
    pub label: CredentialAccountLabel,
    pub vendor_user_ref: DeviceLinkVendorUserRef,
    /// The account's link revision after this link. Bumped on every (re)link,
    /// and a bump is what invalidates any live session bound to the old one.
    pub link_revision: u64,
}

/// One device-link transition, as the host driver reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkStepOutcome {
    pub step: DeviceLinkStep,
    /// Present exactly when `step` is [`DeviceLinkStep::Completed`]. A
    /// completed step without an account is a driver contract violation, and
    /// the auth-side driver terminalizes the flow rather than reporting a
    /// completion it cannot back with a credential.
    pub account: Option<DeviceLinkLinkedAccount>,
}

/// Typed device-link driver failures.
///
/// Sanitized by construction: a vendor's own error text has nowhere to land,
/// only the closed [`DeviceLinkErrorCode`] vocabulary crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceLinkDriverError {
    /// No installed extension binds a device-link adapter for this provider —
    /// the answer to "what does it return when no binding exists". Not
    /// restartable: retrying without installing something cannot succeed.
    #[error("no device-link adapter is bound for provider {provider}")]
    NoBinding { provider: AuthProviderId },
    /// The driver holds no state for this flow — restarted process, reaped
    /// parked link, or a stale card. Restartable: a fresh `begin` works.
    #[error("device-link flow is unknown to the driver")]
    UnknownFlow,
    /// The vendor refused, with a mapped code.
    #[error("device-link vendor call failed ({code:?})")]
    Vendor {
        code: DeviceLinkErrorCode,
        restartable: bool,
    },
    /// Session custody failed, so the link cannot be made durable.
    #[error("device-link custody failed")]
    Custody,
    /// Host mediation is transiently unavailable.
    #[error("device-link host mediation is unavailable")]
    Unavailable,
}

impl DeviceLinkDriverError {
    /// The stable code a card and the audit trail render.
    pub fn code(&self) -> DeviceLinkErrorCode {
        match self {
            // Not `AccountUnavailable`: nothing is wrong with the user's
            // account. The extension is not bound on this deployment, which is
            // an operator condition, and telling the user their account cannot
            // be linked sends them to fix something they do not control.
            Self::NoBinding { .. } => DeviceLinkErrorCode::VendorUnavailable,
            Self::UnknownFlow => DeviceLinkErrorCode::UnknownFlow,
            Self::Vendor { code, .. } => *code,
            Self::Custody => DeviceLinkErrorCode::CustodyFailed,
            Self::Unavailable => DeviceLinkErrorCode::VendorUnavailable,
        }
    }

    /// Whether a fresh `begin` could succeed.
    pub fn restartable(&self) -> bool {
        match self {
            // Installing an extension is not "restarting the flow", and a card
            // that offers "try again" for it lies to the user.
            Self::NoBinding { .. } | Self::Custody => false,
            Self::UnknownFlow | Self::Unavailable => true,
            Self::Vendor { restartable, .. } => *restartable,
        }
    }
}

/// The host-mediated half of a device link, as `ironclaw_auth` sees it.
///
/// **This is the crate's only view of the vendor.** The
/// `DeviceLinkAdapter` an extension implements is invisible from here by
/// design: the extension host resolves provider → bound adapter, scopes the
/// session custody handle, applies rate limits, and answers in this
/// vocabulary. Inside `ironclaw_auth` only this port can be faked, which is
/// what keeps the step machine testable without a vendor.
///
/// Every method is expected to be **serialized per flow** by its
/// implementation. This crate's revision compare-and-swap orders *record*
/// writes; it does not protect a parked vendor connection, and a poll
/// overlapping a submit is a when-not-if race (PROPOSAL §4.3).
#[async_trait]
pub trait DeviceLinkDriver: Send + Sync {
    async fn begin(
        &self,
        request: DeviceLinkBeginRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError>;

    async fn poll(
        &self,
        request: DeviceLinkPollRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError>;

    async fn submit(
        &self,
        request: DeviceLinkSubmitRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError>;

    async fn cancel(&self, request: DeviceLinkCancelRequest) -> Result<(), DeviceLinkDriverError>;
}

pub fn validate_provider_callback_request(
    request: &OAuthProviderCallbackRequest,
) -> Result<(), AuthProductError> {
    if request.authorization_code.expose_secret().trim().is_empty()
        || request.pkce_verifier.expose_secret().trim().is_empty()
    {
        return Err(AuthProductError::MalformedCallback);
    }
    Ok(())
}
