//! The bounded session WebSocket control protocol (`webui.session_event.v1`).
//!
//! The session socket is an event transport, not a command bus: the complete
//! client vocabulary is `subscribe`, `unsubscribe`, and `ping`. Anything else
//! — operation IDs, turn submissions, gate resolutions, unknown frame types,
//! oversized frames — is a protocol violation that closes the connection.
//! Every product mutation stays on authenticated HTTP.

use ironclaw_product_contracts::surface::{ProductStreamSelector, ProductSurfaceError};
use serde::{Deserialize, Serialize};

/// Version tag stamped on every server frame.
pub(crate) const SESSION_EVENT_SCHEMA: &str = "webui.session_event.v1";

/// Hard bound on one client control frame.
pub(crate) const MAX_CLIENT_FRAME_BYTES: usize = 8 * 1024;

/// Hard bound on a client-chosen subscription correlation key.
pub(crate) const MAX_SUBSCRIPTION_ID_BYTES: usize = 64;

/// Hard bound on concurrently active logical subscriptions per socket.
pub(crate) const MAX_ACTIVE_SUBSCRIPTIONS: usize = 16;

/// Bounded queue depth of undelivered event batches per logical subscription.
pub(crate) const SUBSCRIPTION_QUEUE_BATCHES: usize = 16;

/// Bound on a resume cursor supplied in a subscribe frame. Matches the
/// product-side `PROJECTION_CURSOR_MAX_BYTES` plus JSON quoting slack.
pub(crate) const MAX_SUBSCRIBE_CURSOR_BYTES: usize = 2048;

/// Client -> server control frames. This vocabulary is closed; a frame that
/// does not parse into it is a protocol violation.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SessionClientFrame {
    Subscribe {
        subscription_id: String,
        selector: ProductStreamSelector,
        #[serde(default)]
        after_cursor: Option<String>,
    },
    Unsubscribe {
        subscription_id: String,
    },
    Ping,
}

/// Why a client frame was rejected. Rendered into the terminal protocol
/// error frame; carries no client-controlled bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionProtocolViolation {
    FrameTooLarge,
    MalformedFrame,
    SubscriptionIdTooLong,
    CursorTooLong,
    TooManySubscriptions,
    BinaryFrameUnsupported,
}

/// Parse and bound one client text frame.
pub(crate) fn parse_client_frame(
    text: &str,
) -> Result<SessionClientFrame, SessionProtocolViolation> {
    if text.len() > MAX_CLIENT_FRAME_BYTES {
        return Err(SessionProtocolViolation::FrameTooLarge);
    }
    let frame: SessionClientFrame =
        serde_json::from_str(text).map_err(|_| SessionProtocolViolation::MalformedFrame)?;
    match &frame {
        SessionClientFrame::Subscribe {
            subscription_id,
            after_cursor,
            ..
        } => {
            if subscription_id.is_empty() || subscription_id.len() > MAX_SUBSCRIPTION_ID_BYTES {
                return Err(SessionProtocolViolation::SubscriptionIdTooLong);
            }
            if after_cursor
                .as_ref()
                .is_some_and(|cursor| cursor.len() > MAX_SUBSCRIBE_CURSOR_BYTES)
            {
                return Err(SessionProtocolViolation::CursorTooLong);
            }
        }
        SessionClientFrame::Unsubscribe { subscription_id } => {
            if subscription_id.is_empty() || subscription_id.len() > MAX_SUBSCRIPTION_ID_BYTES {
                return Err(SessionProtocolViolation::SubscriptionIdTooLong);
            }
        }
        SessionClientFrame::Ping => {}
    }
    Ok(frame)
}

/// Server -> client frames. Every frame carries the schema tag so the client
/// can ignore vocabularies it does not understand.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionServerFrame {
    Subscribed {
        schema: &'static str,
        subscription_id: String,
        generation: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    },
    Event {
        schema: &'static str,
        subscription_id: String,
        generation: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
        event: serde_json::Value,
    },
    SubscriptionError {
        schema: &'static str,
        subscription_id: String,
        generation: u64,
        error: ironclaw_product_contracts::surface::ProductSurfaceErrorCode,
        kind: ironclaw_product_contracts::surface::ProductSurfaceErrorKind,
        retryable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_cursor: Option<String>,
    },
    Unsubscribed {
        schema: &'static str,
        subscription_id: String,
        generation: u64,
    },
    Pong {
        schema: &'static str,
    },
    ReconnectHint {
        schema: &'static str,
        reason: &'static str,
    },
    ProtocolError {
        schema: &'static str,
        violation: SessionProtocolViolation,
    },
}

impl SessionServerFrame {
    pub(crate) fn subscribed(
        subscription_id: String,
        generation: u64,
        cursor: Option<String>,
    ) -> Self {
        Self::Subscribed {
            schema: SESSION_EVENT_SCHEMA,
            subscription_id,
            generation,
            cursor,
        }
    }

    pub(crate) fn event(
        subscription_id: String,
        generation: u64,
        cursor: Option<String>,
        event: serde_json::Value,
    ) -> Self {
        Self::Event {
            schema: SESSION_EVENT_SCHEMA,
            subscription_id,
            generation,
            cursor,
            event,
        }
    }

    pub(crate) fn subscription_error(
        subscription_id: String,
        generation: u64,
        error: &ProductSurfaceError,
        last_cursor: Option<String>,
    ) -> Self {
        Self::SubscriptionError {
            schema: SESSION_EVENT_SCHEMA,
            subscription_id,
            generation,
            error: error.code,
            kind: error.kind,
            retryable: error.retryable,
            last_cursor,
        }
    }

    pub(crate) fn unsubscribed(subscription_id: String, generation: u64) -> Self {
        Self::Unsubscribed {
            schema: SESSION_EVENT_SCHEMA,
            subscription_id,
            generation,
        }
    }

    pub(crate) fn pong() -> Self {
        Self::Pong {
            schema: SESSION_EVENT_SCHEMA,
        }
    }

    pub(crate) fn lifetime_reconnect_hint() -> Self {
        Self::ReconnectHint {
            schema: SESSION_EVENT_SCHEMA,
            reason: "lifetime_expired",
        }
    }

    pub(crate) fn protocol_error(violation: SessionProtocolViolation) -> Self {
        Self::ProtocolError {
            schema: SESSION_EVENT_SCHEMA,
            violation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_frames_parse_with_typed_selectors() {
        let frame = parse_client_frame(
            r#"{"type":"subscribe","subscription_id":"chat-active","selector":{"kind":"thread","thread_id":"thread-1"},"after_cursor":null}"#,
        )
        .expect("subscribe parses");
        assert_eq!(
            frame,
            SessionClientFrame::Subscribe {
                subscription_id: "chat-active".to_string(),
                selector: ProductStreamSelector::Thread {
                    thread_id: "thread-1".to_string(),
                },
                after_cursor: None,
            }
        );
        assert_eq!(
            parse_client_frame(r#"{"type":"ping"}"#).expect("ping parses"),
            SessionClientFrame::Ping,
        );
    }

    #[test]
    fn mutation_shaped_frames_are_protocol_violations() {
        // Operation IDs, turn submissions, and gate resolutions never gain a
        // WebSocket representation: anything outside the closed control
        // vocabulary is malformed by construction.
        for hostile in [
            r#"{"type":"invoke","operation_id":"webui.submit_turn.v1","input":{}}"#,
            r#"{"type":"submit_turn","thread_id":"t","text":"hi"}"#,
            r#"{"type":"resolve_gate","gate_ref":"g"}"#,
            r#"{"type":"subscribe","subscription_id":"a","selector":{"kind":"thread","thread_id":"t"},"operation_id":"webui.submit_turn.v1"}"#,
            r#"{"type":"query","view_id":"webui.threads.v1"}"#,
            "not json",
        ] {
            assert_eq!(
                parse_client_frame(hostile),
                Err(SessionProtocolViolation::MalformedFrame),
                "hostile frame must be rejected: {hostile}",
            );
        }
    }

    #[test]
    fn client_frame_bounds_are_enforced() {
        let oversized_id = "a".repeat(MAX_SUBSCRIPTION_ID_BYTES + 1);
        let frame = format!(
            r#"{{"type":"subscribe","subscription_id":"{oversized_id}","selector":{{"kind":"thread","thread_id":"t"}}}}"#
        );
        assert_eq!(
            parse_client_frame(&frame),
            Err(SessionProtocolViolation::SubscriptionIdTooLong),
        );

        let oversized_cursor = "c".repeat(MAX_SUBSCRIBE_CURSOR_BYTES + 1);
        let frame = format!(
            r#"{{"type":"subscribe","subscription_id":"chat","selector":{{"kind":"thread","thread_id":"t"}},"after_cursor":"{oversized_cursor}"}}"#
        );
        assert_eq!(
            parse_client_frame(&frame),
            Err(SessionProtocolViolation::CursorTooLong),
        );

        let padding = " ".repeat(MAX_CLIENT_FRAME_BYTES);
        let frame = format!(r#"{{"type":"ping"}}{padding}"#);
        assert_eq!(
            parse_client_frame(&frame),
            Err(SessionProtocolViolation::FrameTooLarge),
        );

        assert_eq!(
            parse_client_frame(r#"{"type":"unsubscribe","subscription_id":""}"#),
            Err(SessionProtocolViolation::SubscriptionIdTooLong),
        );
    }

    #[test]
    fn server_frames_carry_the_versioned_schema_tag() {
        let frame = SessionServerFrame::subscribed("chat".to_string(), 3, Some("\"c\"".into()));
        let value = serde_json::to_value(&frame).expect("frame serializes");
        assert_eq!(value["schema"], SESSION_EVENT_SCHEMA);
        assert_eq!(value["type"], "subscribed");
        assert_eq!(value["generation"], 3);

        let pong = serde_json::to_value(SessionServerFrame::pong()).expect("pong serializes");
        assert_eq!(pong["schema"], SESSION_EVENT_SCHEMA);

        let hint = serde_json::to_value(SessionServerFrame::lifetime_reconnect_hint())
            .expect("hint serializes");
        assert_eq!(hint["reason"], "lifetime_expired");
    }
}
