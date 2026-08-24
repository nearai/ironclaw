//! The **device-link** auth hook: the one auth method whose mechanics an
//! extension implements instead of declaring.
//!
//! Every other auth method here is data the host engine executes — an
//! `oauth2_code` recipe names endpoints and pointers, and vendors differ only
//! in parameters. A device-link handshake is not expressible that way: it is a
//! multi-round protocol conversation with the vendor, driven from a live
//! connection the extension owns. So the recipe
//! ([`crate::recipe::DeviceLinkRecipe`]) carries **display metadata only**, and
//! everything mechanical arrives through [`DeviceLinkAdapter`].
//!
//! **What that costs, said plainly.** The no-auth-adapter rule existed to close
//! a class of attack surface — parameter override, state tampering, credential
//! exfiltration — and this hook re-opens part of it. Nothing the host can check
//! binds a [`DeviceLinkStep::Display`] payload to a token the host itself
//! exported: the host does not speak the vendor protocol and is structurally
//! incapable of verifying it. A compromised adapter can display a payload for a
//! session it controls, and on the identifier path it receives the complete
//! credential set the user types. The compensations are that the implementing
//! package is first-party and reviewed in-tree, and that after completion the
//! user is shown the resolved account so a substituted login is *observable*.
//! That is detection, not prevention. This module must not be read as claiming
//! more.
//!
//! Secrets — login codes and account passwords — reach the adapter as
//! [`SecretString`], are never echoed into a [`DeviceLinkStep`], never into a
//! [`DeviceLinkError`], and never appear in `Debug`. [`DeviceLinkInput`]
//! deliberately does not implement `Serialize`: a step or an event physically
//! cannot carry one.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::error::HostApiError;
use ironclaw_host_api::ids::{ExtensionId, UserId};

use crate::linked_session::{LinkedAccountGrant, LinkedSessionError, LinkedSessionPort};

/// Maximum length of a [`DeviceLinkPayload`], in bytes. Sized against the
/// practical capacity of a scannable code rather than a protocol limit — a
/// payload larger than this cannot be rendered as one anyway.
pub const MAX_DEVICE_LINK_PAYLOAD_BYTES: usize = 4 * 1024;

/// Maximum length of a submitted identifier (the non-secret one), in bytes.
pub const MAX_DEVICE_LINK_IDENTIFIER_BYTES: usize = 128;

/// Maximum length of a submitted secret (login code, account password), in
/// bytes. Bounds an oversized paste before it reaches vendor code.
pub const MAX_DEVICE_LINK_SECRET_BYTES: usize = 1024;

/// Maximum length of a bounded display string in a step (labels, hints,
/// account labels, vendor refs), in bytes.
pub const MAX_DEVICE_LINK_LABEL_BYTES: usize = 512;

/// Which linking path the user chose.
///
/// Two paths, named structurally rather than by mechanism, because the
/// mechanism is the vendor's: an extension might offer a scannable code and a
/// number-plus-code fallback, or something else entirely. The recipe supplies
/// the labels a user reads; this enum only says *which* of the extension's two
/// declared paths to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLinkMode {
    /// The extension's primary path.
    Default,
    /// The extension's declared fallback. An extension that declares no
    /// alternate rejects this with [`DeviceLinkError::UnsupportedMode`].
    Alternate,
}

/// What the card is asking the user for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLinkInputKind {
    /// A non-secret account identifier (a phone number, a handle).
    Identifier,
    /// A one-time login code the vendor delivered out of band.
    Code,
    /// A standing account password (a second factor held by the vendor).
    Password,
}

/// How a [`DeviceLinkStep::Display`] payload is meant to be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLinkDisplayKind {
    /// Render as a scannable QR code.
    QrCode,
    /// Render as a link the user opens on the device being linked.
    Link,
}

/// Stable failure codes a card and the audit trail can both reason about.
///
/// Closed on purpose: the host must be able to tell "ask again" from "this can
/// never succeed" without reading vendor text. Vendor detail stays vendor-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLinkErrorCode {
    /// The displayed payload or the flow's own window elapsed.
    Expired,
    /// No such flow — restarted process, reaped flow, or a stale card.
    UnknownFlow,
    /// The user (or the vendor, on the user's behalf) refused the device.
    Declined,
    /// The submitted identifier, code, or password was rejected.
    InvalidInput,
    /// The vendor is rate-limiting; back off and retry.
    RateLimited,
    /// The **host** is rate-limiting, not the vendor.
    ///
    /// A distinct code because the two are different facts with different
    /// remedies, and folding host budgets into `RateLimited` told a user (and
    /// the audit trail) that a vendor pushed back when no vendor was called.
    /// Restartable once the window rolls.
    HostThrottled,
    /// A ceiling the user can act on has been reached — a vendor's cap on
    /// linked devices, or a host cap on concurrent links. Distinct from
    /// throttling: waiting does not help, removing something does.
    LimitReached,
    /// The account cannot be linked at all — deactivated, banned, ineligible.
    /// Terminal: re-prompting forever is the wrong behavior.
    AccountUnavailable,
    /// This external identity is already connected, either to another product
    /// user or as a different identity for this user. Terminal until the
    /// existing link is removed from the account that owns it.
    IdentityConflict,
    /// The vendor failed transiently.
    VendorUnavailable,
    /// Host-side custody failed (see [`LinkedSessionError`]).
    CustodyFailed,
    /// Anything else. Reserved for genuinely unclassifiable failures.
    Internal,
}

/// A bounded, validated payload the card renders — a scannable code's content
/// or a link.
///
/// **Treated as sensitive.** On the scan path this value *is* the login token:
/// anything that can display it can invite a device onto the account. It
/// redacts under `Debug` for the same reason a credential does, and the single
/// read accessor is named [`DeviceLinkPayload::expose`] so every render site is
/// greppable.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeviceLinkPayload(String);

impl DeviceLinkPayload {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    /// The payload text. Named `expose` because rendering it is a security
    /// decision, not a formatting one.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always `false` — [`DeviceLinkPayload::new`] rejects an empty payload.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn validate(value: &str) -> Result<(), HostApiError> {
        // The value is redacted from the error too: a rejected payload is
        // still a payload.
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "device_link_payload",
            value: "<redacted>".to_string(),
            reason: reason.to_string(),
        };
        if value.is_empty() {
            return Err(invalid("must not be empty"));
        }
        if value.len() > MAX_DEVICE_LINK_PAYLOAD_BYTES {
            return Err(invalid("exceeds the maximum device-link payload length"));
        }
        if value.chars().any(char::is_control) {
            return Err(invalid("must not contain control characters"));
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid("must not contain whitespace"));
        }
        Ok(())
    }
}

impl TryFrom<String> for DeviceLinkPayload {
    type Error = HostApiError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DeviceLinkPayload> for String {
    fn from(payload: DeviceLinkPayload) -> Self {
        payload.0
    }
}

impl fmt::Debug for DeviceLinkPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceLinkPayload({} bytes, redacted)", self.0.len())
    }
}

/// One value the user submitted into an in-progress link.
///
/// Not `Serialize`, and not `Debug`-transparent: the two secret variants must
/// be unable to reach a durable record, a log line, or a projection. The host
/// calls [`DeviceLinkInput::validate`] before handing one to an adapter, so
/// vendor code never sees an unbounded paste.
pub enum DeviceLinkInput {
    /// A non-secret account identifier. Bounded, not hidden.
    Identifier(String),
    /// A one-time login code.
    Code(SecretString),
    /// A standing account password.
    Password(SecretString),
}

impl DeviceLinkInput {
    /// Which prompt this input answers.
    pub fn kind(&self) -> DeviceLinkInputKind {
        match self {
            Self::Identifier(_) => DeviceLinkInputKind::Identifier,
            Self::Code(_) => DeviceLinkInputKind::Code,
            Self::Password(_) => DeviceLinkInputKind::Password,
        }
    }

    /// Bound the submitted value. The host runs this at the boundary, before
    /// dispatch, so an oversized or control-laden paste is rejected without
    /// vendor code ever seeing it.
    pub fn validate(&self) -> Result<(), DeviceLinkError> {
        use secrecy::ExposeSecret as _;

        let too_long = |kind| DeviceLinkError::InvalidInput {
            kind,
            reason: "submitted value exceeds its length bound",
        };
        let empty = |kind| DeviceLinkError::InvalidInput {
            kind,
            reason: "submitted value must not be empty",
        };
        match self {
            Self::Identifier(value) => {
                if value.is_empty() {
                    return Err(empty(DeviceLinkInputKind::Identifier));
                }
                if value.len() > MAX_DEVICE_LINK_IDENTIFIER_BYTES {
                    return Err(too_long(DeviceLinkInputKind::Identifier));
                }
                if value.chars().any(char::is_control) {
                    return Err(DeviceLinkError::InvalidInput {
                        kind: DeviceLinkInputKind::Identifier,
                        reason: "submitted value must not contain control characters",
                    });
                }
            }
            Self::Code(secret) | Self::Password(secret) => {
                let kind = self.kind();
                let exposed = secret.expose_secret();
                if exposed.is_empty() {
                    return Err(empty(kind));
                }
                if exposed.len() > MAX_DEVICE_LINK_SECRET_BYTES {
                    return Err(too_long(kind));
                }
            }
        }
        Ok(())
    }
}

impl fmt::Debug for DeviceLinkInput {
    /// Kind only. The identifier is bounded rather than secret, but it is
    /// still a user's account handle, so it does not print either.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceLinkInput({:?}, redacted)", self.kind())
    }
}

/// Where an in-progress link stands after one adapter call.
///
/// Wire vocabulary: the host persists a step onto durable flow state and
/// projects it into a card, so every variant here has to survive
/// serialization. Durations ride the wire as whole milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum DeviceLinkStep {
    /// Show the user something to scan or open, valid for `expires_in`.
    Display {
        kind: DeviceLinkDisplayKind,
        payload: DeviceLinkPayload,
        #[serde(with = "duration_millis")]
        expires_in: Duration,
    },
    /// Nothing to show; poll again after `retry_in`.
    AwaitingVendor {
        #[serde(with = "duration_millis")]
        retry_in: Duration,
    },
    /// Ask the user for one value.
    InputRequired {
        kind: DeviceLinkInputKind,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
    /// The account is linked. `vendor_user_ref` is shown to the user so a
    /// substituted login is visible rather than silent.
    Completed {
        account_label: String,
        vendor_user_ref: String,
    },
    /// The attempt ended. `restartable` says whether a fresh `begin` could
    /// succeed — a rate limit can, a deactivated account cannot.
    Failed {
        code: DeviceLinkErrorCode,
        restartable: bool,
    },
}

impl DeviceLinkStep {
    /// The step's discriminator, for a projection that needs the shape without
    /// the payload.
    pub fn kind(&self) -> DeviceLinkStepKind {
        match self {
            Self::Display { .. } => DeviceLinkStepKind::Display,
            Self::AwaitingVendor { .. } => DeviceLinkStepKind::AwaitingVendor,
            Self::InputRequired { .. } => DeviceLinkStepKind::InputRequired,
            Self::Completed { .. } => DeviceLinkStepKind::Completed,
            Self::Failed { .. } => DeviceLinkStepKind::Failed,
        }
    }

    /// Whether the flow has stopped advancing.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. })
    }

    /// Bound the display strings a step carries. The host runs this on an
    /// adapter's return value, so a vendor cannot push unbounded text into a
    /// durable record or a card.
    pub fn validate(&self) -> Result<(), DeviceLinkError> {
        fn check(value: &str) -> Result<(), DeviceLinkError> {
            if value.is_empty() {
                return Err(DeviceLinkError::InvalidStep {
                    reason: "step display text must not be empty",
                });
            }
            if value.len() > MAX_DEVICE_LINK_LABEL_BYTES {
                return Err(DeviceLinkError::InvalidStep {
                    reason: "step display text exceeds its length bound",
                });
            }
            if value
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err(DeviceLinkError::InvalidStep {
                    reason: "step display text contains unsupported control characters",
                });
            }
            Ok(())
        }

        match self {
            Self::Display { .. } | Self::AwaitingVendor { .. } | Self::Failed { .. } => Ok(()),
            Self::InputRequired { label, hint, .. } => {
                check(label)?;
                if let Some(hint) = hint {
                    check(hint)?;
                }
                Ok(())
            }
            Self::Completed {
                account_label,
                vendor_user_ref,
            } => {
                check(account_label)?;
                check(vendor_user_ref)
            }
        }
    }
}

/// [`DeviceLinkStep`]'s discriminator, so a projection can name the shape a
/// card is in without carrying the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLinkStepKind {
    Display,
    AwaitingVendor,
    InputRequired,
    Completed,
    Failed,
}

/// Host-issued identity for one in-progress device-link flow.
///
/// Opaque, like [`crate::linked_session::LinkedAccountRef`] and for the same
/// reason: the durable flow record belongs to the auth domain, which sits above
/// this floor and which an extension package may not name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceLinkFlowId(String);

impl DeviceLinkFlowId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "device_link_flow_id",
            value: value.clone(),
            reason: reason.to_string(),
        };
        if value.is_empty() {
            return Err(invalid("must not be empty"));
        }
        if value.len() > 128 {
            return Err(invalid("exceeds the maximum device-link flow-id length"));
        }
        if value.chars().any(char::is_control) {
            return Err(invalid("must not contain control characters"));
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid("must not contain whitespace"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceLinkFlowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Everything an adapter is allowed to know about the call it is serving.
///
/// The host builds it per call and scopes it: `session` addresses exactly one
/// credential account, so the adapter cannot reach another user's custody
/// through it, and `config` carries manifest/administrator values that are
/// never secret — secret material reaches an adapter only as a
/// [`DeviceLinkInput`], from the human in front of the card.
pub struct DeviceLinkContext<'a> {
    /// The flow this call advances.
    pub flow_id: &'a DeviceLinkFlowId,
    /// The extension whose adapter is being called.
    pub extension_id: &'a ExtensionId,
    /// The user linking their account.
    pub user_id: &'a UserId,
    /// Non-secret configuration resolved from the manifest and administrator
    /// settings.
    pub config: &'a BTreeMap<String, String>,
    /// Pre-scoped session custody for this flow's account.
    pub session: &'a dyn LinkedSessionPort,
    /// The account this call acts on, once one exists.
    ///
    /// `None` while a link is still being established — there is no credential
    /// account until custody is durable. `Some` on
    /// [`DeviceLinkAdapter::revoke`] and on any later call, which is what lets
    /// an adapter evict its own live state for exactly that account and
    /// revision without holding a handle to every user's.
    pub account: Option<&'a LinkedAccountGrant>,
}

impl fmt::Debug for DeviceLinkContext<'_> {
    /// Identity only. `config` may name administrator-entered values and
    /// `session` is a live custody handle; neither belongs in a diagnostic.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceLinkContext")
            .field("flow_id", &self.flow_id)
            .field("extension_id", &self.extension_id)
            .field("user_id", &self.user_id)
            .field("config_keys", &self.config.len())
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

/// Typed device-link failures.
///
/// Every free-text field is `&'static str` on purpose: an adapter can only
/// pick from fixed, host-readable phrases, so a code, a password, or a vendor
/// error body has nowhere to be interpolated. Vendor specifics ride
/// [`DeviceLinkErrorCode`] instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceLinkError {
    /// The adapter has no state for this flow. Restartable.
    #[error("device-link flow is unknown to the adapter")]
    UnknownFlow,
    /// The submitted value was rejected before it reached the vendor.
    #[error("device-link input was rejected: {reason}")]
    InvalidInput {
        kind: DeviceLinkInputKind,
        reason: &'static str,
    },
    /// An adapter returned a step the host will not accept.
    #[error("device-link step was rejected: {reason}")]
    InvalidStep { reason: &'static str },
    /// The extension does not offer the requested path.
    #[error("device-link mode is not offered by this extension")]
    UnsupportedMode { mode: DeviceLinkMode },
    /// The vendor refused, with a mapped code. `restartable` distinguishes
    /// "try again" from "this account can never link".
    #[error("device-link vendor call failed ({code:?})")]
    Vendor {
        code: DeviceLinkErrorCode,
        restartable: bool,
    },
    /// Custody failed; the link cannot be made durable.
    #[error("device-link custody failed")]
    Custody(#[from] LinkedSessionError),
    /// The adapter failed for a reason the run cannot recover from.
    #[error("device-link adapter failed: {reason}")]
    Internal { reason: &'static str },
}

impl DeviceLinkError {
    /// The stable code a card and the audit trail render.
    pub fn code(&self) -> DeviceLinkErrorCode {
        match self {
            Self::UnknownFlow => DeviceLinkErrorCode::UnknownFlow,
            Self::InvalidInput { .. } | Self::InvalidStep { .. } => {
                DeviceLinkErrorCode::InvalidInput
            }
            Self::UnsupportedMode { .. } | Self::Internal { .. } => DeviceLinkErrorCode::Internal,
            Self::Vendor { code, .. } => *code,
            Self::Custody(_) => DeviceLinkErrorCode::CustodyFailed,
        }
    }

    /// Whether a fresh [`DeviceLinkAdapter::begin`] could succeed.
    pub fn restartable(&self) -> bool {
        match self {
            Self::UnknownFlow | Self::InvalidInput { .. } => true,
            Self::InvalidStep { .. } | Self::UnsupportedMode { .. } | Self::Internal { .. } => {
                false
            }
            Self::Vendor { restartable, .. } => *restartable,
            Self::Custody(_) => false,
        }
    }
}

/// The vendor half of a device-link flow.
///
/// # Obligations an implementor must satisfy
///
/// These are contract, not advice, and the host cannot check them for you:
///
/// * **Custody before completion.** Anything a later tool call needs — the
///   session credential above all — must be written through the supplied
///   [`crate::linked_session::LinkedSessionPort`] *before* a
///   [`DeviceLinkStep::Completed`] is returned. The host mints the credential
///   account from that step, so a blob written afterwards races a call that
///   can already be dispatched.
/// * **`poll` is a read while awaiting the user.** While the flow waits on
///   something the *user* must do, `poll` must not advance vendor state. It
///   may still talk to the vendor — a protocol whose acceptance arrives only
///   by re-asking has no other way to notice — but it must be safe to call at
///   the host's cadence, on every card that happens to be open, without
///   consuming a one-shot.
/// * **Completion is provisional until `finalize`.** Returning
///   [`DeviceLinkStep::Completed`] means the vendor authorized the device and
///   custody contains the session, but the adapter must retain enough state
///   for [`DeviceLinkAdapter::cancel`] to undo that authorization until the
///   host calls [`DeviceLinkAdapter::finalize`].
/// * **`cancel` and `finalize` are idempotent** and safe on a flow that is
///   already gone.
/// * **A `begin` naming a flow that is already live is a re-mint**, not a
///   second attempt: the host asks for it when a displayed frame's clock
///   lapses, and it must be answered with a fresh frame for the same link.
///
/// [`ironclaw_auth::test_support::conformance::assert_device_link_driver_conformance`]
/// exercises the host-side half of these against a real implementation.
///
/// The host owns the state machine, the revision compare-and-swap, the TTLs,
/// the rate limits, and the credential lifecycle. The adapter owns exactly the
/// protocol conversation, and holds any parked connection a login is bound to
/// between calls.
///
/// `cancel` exists so an abort can be *awaited*: once the vendor has authorized
/// the device, walking away leaves a live authorization the host has forgotten,
/// and a synchronous `Drop` cannot make an asynchronous logout call.
#[async_trait]
pub trait DeviceLinkAdapter: Send + Sync {
    /// Start a link on the chosen path.
    async fn begin(
        &self,
        ctx: &DeviceLinkContext<'_>,
        mode: DeviceLinkMode,
    ) -> Result<DeviceLinkStep, DeviceLinkError>;

    /// Advance a link that is waiting on the vendor. Must be a pure read while
    /// the flow is waiting on the user: the host polls it concurrently with
    /// whatever the user is typing.
    async fn poll(&self, ctx: &DeviceLinkContext<'_>) -> Result<DeviceLinkStep, DeviceLinkError>;

    /// Submit one value the previous step asked for.
    async fn submit_input(
        &self,
        ctx: &DeviceLinkContext<'_>,
        input: DeviceLinkInput,
    ) -> Result<DeviceLinkStep, DeviceLinkError>;

    /// Accept a completed provisional link after the host has durably minted
    /// its credential account and any product identity bindings. This only
    /// relinquishes the adapter's rollback state; it must not log the device
    /// out.
    async fn finalize(&self, ctx: &DeviceLinkContext<'_>);

    /// Abandon an in-progress link, undoing any authorization it already
    /// obtained, including one behind a provisional `Completed` step the host
    /// could not commit.
    async fn cancel(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError>;

    /// Tear down an established link: end the vendor-side authorization and
    /// drop any live state for the account the context names.
    async fn revoke(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError>;
}

/// `Duration` as whole milliseconds on the wire. serde's own `Duration`
/// representation is a `{secs, nanos}` struct, which no card would want to
/// read and no schema would want to pin.
mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        value: &Duration,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(u64::try_from(value.as_millis()).unwrap_or(u64::MAX))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }
}

#[cfg(test)]
mod tests;
