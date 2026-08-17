//! The fifteen bound standard messaging operations, split by what they act on.
//!
//! Each op is one `async fn` taking the caller's live [`PooledSession`] and the
//! already-schema-validated canonical input, and returning canonical output or
//! a [`ToolError`]. Routing by capability id is [`super::tools`]; the Telegram
//! ⇄ canonical shape work is [`super::mapping`]; the socket and its retry
//! discipline are [`super::transport`]. Nothing here opens a connection,
//! resolves an account, or touches custody.
//!
//! **`get_thread_replies` is deliberately unbound.** Telegram keeps thread
//! replies in the same chat history, so `get_conversation_history` already
//! returns them and a separate op would have nothing distinct to fetch. The
//! deviation from the canonical description ("history excludes thread replies
//! where available") is stated in the vendor addendum.

use ironclaw_extension_contracts::tool_adapter::ToolError;
use ironclaw_host_api::messaging::StandardMessagingErrorCode;
use serde_json::Value;

use super::{
    MAX_ITEMS_PER_PAGE,
    mapping::{self, ConversationRef, Cursor, OpFamily},
    pool::{PooledSession, SessionPoolError},
    transport::VendorOpKind,
};

pub(crate) mod people;
pub(crate) mod reads;
pub(crate) mod writes;

// How the tool half learns which linked account a call acts as: the
// host-implemented `LinkedAccountResolver` supplied at bind
// (`ironclaw_extension_contracts::linked_session`), beside the custody
// factory. PROPOSAL §5.1 settles that containment must be rooted in a
// host-minted `LinkedAccountGrant` — explicitly *not* in
// `ToolCall.scope.user_id` — and §8.3 freezes `ToolPorts`/`ToolCall`, so the
// bind-time port is the one seam. An implementation in this package that read
// `scope.user_id` and minted its own grant would reintroduce the hazard the
// design refuses, and must not be written.

/// Project a resolution failure onto the tool vocabulary (a `From` impl is
/// unavailable here: both types are foreign).
pub(crate) fn resolution_error(
    error: ironclaw_extension_contracts::linked_session::LinkedAccountResolutionError,
) -> ToolError {
    use ironclaw_extension_contracts::linked_session::LinkedAccountResolutionError;
    match error {
        LinkedAccountResolutionError::NotLinked => mapping::auth_required(),
        LinkedAccountResolutionError::Unavailable => {
            mapping::failed(StandardMessagingErrorCode::VendorError)
        }
    }
}

impl From<SessionPoolError> for ToolError {
    fn from(error: SessionPoolError) -> Self {
        match error {
            // Unlinked or re-linked while this work was in flight, or never
            // linked at all: both are things the user fixes by linking, so they
            // park on the auth gate rather than teaching the model that
            // Telegram rejected its arguments.
            SessionPoolError::Revoked | SessionPoolError::NotLinked => mapping::auth_required(),
            // Deployment-shaped, not model- or user-correctable: carry the
            // honest sentence so the failure is attributable without a vendor
            // call ever having happened.
            SessionPoolError::NotConfigured => {
                mapping::failed_because(StandardMessagingErrorCode::VendorError, error.to_string())
            }
            SessionPoolError::Custody(_)
            | SessionPoolError::AtCapacity
            | SessionPoolError::Poisoned => {
                mapping::failed(StandardMessagingErrorCode::VendorError)
            }
        }
    }
}

/// Runs one vendor call against the caller's session.
///
/// Two rules live here and nowhere else in this module tree:
///
/// * **The generation fence is checked before every RPC**, not only at pool
///   lookup. Checking only on lookup means unlink either stalls for a whole
///   `TOOL_CALL_TIMEOUT` or lets a send land after the user pressed unlink
///   (PROPOSAL §4.3). A write already on the wire cannot be recalled — that
///   residue is disclosed, not closed.
/// * **A write is never repeated.** `super::transport::MtprotoConnection`
///   states the same rule for the raw-TL path and installs `NoRetries` on the
///   client this helper calls through, so neither layer can silently re-send;
///   the read retry below is the only repetition in either.
pub(crate) async fn vendor_call<T, F, Fut>(
    session: &PooledSession,
    family: OpFamily,
    mut call: F,
) -> Result<T, ToolError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, grammers_client::InvocationError>>,
{
    let kind = match family {
        OpFamily::Write => VendorOpKind::Write,
        OpFamily::Read | OpFamily::People => VendorOpKind::Read,
    };
    let connection = session.connection();
    let mut attempt = 0;
    loop {
        attempt += 1;
        session.check_fence()?;
        session.touch();
        let error = match call().await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let retryable_transport = matches!(
            error,
            grammers_client::InvocationError::Io(_)
                | grammers_client::InvocationError::Dropped
                | grammers_client::InvocationError::Transport(_)
        );
        if kind == VendorOpKind::Read
            && retryable_transport
            && attempt < 2
            && connection.is_runner_alive()
        {
            continue;
        }
        return Err(mapping::map_vendor_error(family, &error));
    }
}

// ---------------------------------------------------------------------------
// Canonical input readers
// ---------------------------------------------------------------------------
//
// Input arrives schema-validated, so these read shapes rather than re-validate
// them. What they do NOT do is paper over a missing field with a default: a
// canonical input that does not carry what its schema promises is a host bug,
// and silently substituting an empty string would send a message nobody asked
// for.

fn missing_field(field: &str) -> ToolError {
    ToolError::Failed {
        kind: ironclaw_host_api::dispatch::RuntimeDispatchErrorKind::Unknown,
        safe_summary: Some(format!(
            "the telegram adapter received input without its required {field} field"
        )),
        model_visible_cause: None,
    }
}

pub(crate) fn input_str<'a>(input: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| missing_field(field))
}

pub(crate) fn optional_str<'a>(input: &'a Value, field: &str) -> Option<&'a str> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// Reads the `conversation` argument and rehydrates it to a usable peer.
///
/// A ref that does not decode is `messaging.unknown_conversation`, the same
/// answer a migrated basic group and a `CHANNEL_PRIVATE` channel get: Telegram
/// deliberately does not distinguish "gone" from "you cannot see it", so
/// `not_a_member` would claim knowledge nobody has.
pub(crate) fn conversation_arg(input: &Value, field: &str) -> Result<ConversationRef, ToolError> {
    let raw = input_str(input, field)?;
    ConversationRef::decode(raw)
        .ok_or_else(|| mapping::failed(StandardMessagingErrorCode::UnknownConversation))
}

/// Reads the canonical `message_ref` argument.
pub(crate) fn message_ref_arg(input: &Value) -> Result<(ConversationRef, i32), ToolError> {
    let value = input
        .get("message_ref")
        .filter(|value| value.is_object())
        .ok_or_else(|| missing_field("message_ref"))?;
    mapping::parse_message_ref(value)
        .ok_or_else(|| mapping::failed(StandardMessagingErrorCode::UnknownMessage))
}

/// Reads the optional `limit`, clamped to this adapter's per-page ceiling
/// ([`MAX_ITEMS_PER_PAGE`], PROPOSAL §6.4/§7.2).
///
/// The clamp is a ceiling on *untrusted content volume*, not a convenience: the
/// canonical input schemas bound `limit` loosely or not at all, so an
/// unclamped page is however many attacker-authored messages the model happened
/// to ask for.
pub(crate) fn limit_arg(input: &Value) -> usize {
    input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|limit| limit as usize)
        .unwrap_or(MAX_ITEMS_PER_PAGE)
        .clamp(1, MAX_ITEMS_PER_PAGE)
}

/// Reads the optional `cursor`.
///
/// A supplied cursor that does not decode is `messaging.unsupported_content`
/// (kind: input) — **never** a silent restart at page one, which looks like
/// success while re-reading content the caller already has.
pub(crate) fn cursor_arg(input: &Value) -> Result<Option<Cursor>, ToolError> {
    match optional_str(input, "cursor") {
        None => Ok(None),
        Some(raw) => Cursor::decode(raw)
            .map(Some)
            .ok_or_else(|| mapping::failed(StandardMessagingErrorCode::UnsupportedContent)),
    }
}

/// Refuses a cursor on an op whose vendor listing cannot be resumed.
///
/// Telegram's dialog and participant listings expose no offset this adapter can
/// carry across calls, so those ops emit no `next_cursor` at all. If one is
/// supplied anyway it is rejected rather than ignored — ignoring it would
/// silently return page one under the name of page two.
pub(crate) fn reject_cursor(input: &Value) -> Result<(), ToolError> {
    match optional_str(input, "cursor") {
        None => Ok(()),
        Some(_) => Err(mapping::failed_because(
            StandardMessagingErrorCode::UnsupportedContent,
            "this telegram listing is not paginated; raise limit instead of passing a cursor"
                .to_string(),
        )),
    }
}

#[cfg(test)]
mod tests;
