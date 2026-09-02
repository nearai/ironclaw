//! The session event stream vocabulary (`webui.session_event.v1`).
//!
//! `POST /api/webchat/v2/session/events` opens one `text/event-stream` per
//! authenticated page carrying every logical subscription the request body
//! names, each with its own resume cursor. The stream is an event transport,
//! not a command bus: nothing travels client→server after the request body,
//! every product mutation stays on authenticated HTTP, and changing the
//! subscription set means reconnecting with each selector's last cursor.

use std::collections::BTreeSet;

use ironclaw_product_contracts::surface::{
    ProductStreamSelector, ProductSurfaceError, ProductSurfaceValidationCode,
};
use serde::{Deserialize, Serialize};

/// Version tag stamped on every server frame.
pub(crate) const SESSION_EVENT_SCHEMA: &str = "webui.session_event.v1";

/// Hard bound on a client-chosen subscription correlation key.
pub(crate) const MAX_SUBSCRIPTION_ID_BYTES: usize = 64;

/// Hard bound on logical subscriptions per stream.
pub(crate) const MAX_ACTIVE_SUBSCRIPTIONS: usize = 16;

/// Per-subscription queue depth (event batches) between a driver task and
/// the stream writer: a slow client backpressures its own subscriptions
/// instead of growing memory.
pub(crate) const SUBSCRIPTION_QUEUE_BATCHES: usize = 16;

/// Hard bound on one resume cursor token.
pub(crate) const MAX_SUBSCRIBE_CURSOR_BYTES: usize = 2048;

/// The request body of the session event stream: the complete subscription
/// set for this connection.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEventsRequest {
    pub subscriptions: Vec<SessionSubscriptionRequest>,
}

/// One logical subscription: a client-chosen correlation id, the typed
/// selector the surface authorizes, and the cursor to resume strictly after.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSubscriptionRequest {
    pub subscription_id: String,
    pub selector: ProductStreamSelector,
    #[serde(default)]
    pub after_cursor: Option<String>,
}

impl SessionEventsRequest {
    /// Bound the request before any subscription task exists: one to
    /// [`MAX_ACTIVE_SUBSCRIPTIONS`] subscriptions with distinct, bounded ids
    /// and bounded cursors. Selector authorization happens per subscription
    /// on the product surface, never here.
    pub(crate) fn validate(&self) -> Result<(), ProductSurfaceError> {
        if self.subscriptions.is_empty() || self.subscriptions.len() > MAX_ACTIVE_SUBSCRIPTIONS {
            return Err(invalid("subscriptions"));
        }
        let mut seen = BTreeSet::new();
        for subscription in &self.subscriptions {
            let id = subscription.subscription_id.as_str();
            if id.is_empty() || id.len() > MAX_SUBSCRIPTION_ID_BYTES || !seen.insert(id) {
                return Err(invalid("subscription_id"));
            }
            if subscription
                .after_cursor
                .as_ref()
                .is_some_and(|cursor| cursor.len() > MAX_SUBSCRIBE_CURSOR_BYTES)
            {
                return Err(invalid("after_cursor"));
            }
        }
        Ok(())
    }
}

fn invalid(field: &'static str) -> ProductSurfaceError {
    ProductSurfaceError::validation(field, ProductSurfaceValidationCode::InvalidValue)
}

/// Server → client frames. Every frame carries the schema tag so a client
/// can ignore vocabularies it does not understand; each travels as one SSE
/// event whose `event:` name is the frame type.
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
    ReconnectHint {
        schema: &'static str,
        reason: &'static str,
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

    pub(crate) fn lifetime_reconnect_hint() -> Self {
        Self::ReconnectHint {
            schema: SESSION_EVENT_SCHEMA,
            reason: "lifetime_expired",
        }
    }

    /// The SSE `event:` name this frame travels under (its `type` tag).
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Subscribed { .. } => "subscribed",
            Self::Event { .. } => "event",
            Self::SubscriptionError { .. } => "subscription_error",
            Self::ReconnectHint { .. } => "reconnect_hint",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(subscriptions: Vec<(&str, Option<String>)>) -> SessionEventsRequest {
        SessionEventsRequest {
            subscriptions: subscriptions
                .into_iter()
                .map(|(id, after_cursor)| SessionSubscriptionRequest {
                    subscription_id: id.to_string(),
                    selector: ProductStreamSelector::Thread {
                        thread_id: "thread-1".to_string(),
                    },
                    after_cursor,
                })
                .collect(),
        }
    }

    #[test]
    fn request_bodies_parse_with_typed_selectors_and_reject_unknown_fields() {
        let parsed: SessionEventsRequest = serde_json::from_str(
            r#"{"subscriptions":[{"subscription_id":"chat","selector":{"kind":"thread","thread_id":"t"},"after_cursor":null},{"subscription_id":"rc","selector":{"kind":"run_completions"}}]}"#,
        )
        .expect("body parses");
        assert_eq!(parsed.subscriptions.len(), 2);
        assert_eq!(
            parsed.subscriptions[1].selector,
            ProductStreamSelector::RunCompletions
        );
        // Operation ids, turn submissions, and gate resolutions never gain a
        // stream representation: anything outside the body shape is rejected.
        for hostile in [
            r#"{"subscriptions":[],"operation_id":"webui.submit_turn.v1"}"#,
            r#"{"subscriptions":[{"subscription_id":"a","selector":{"kind":"thread","thread_id":"t"},"input":{}}]}"#,
            r#"{"type":"submit_turn","thread_id":"t","text":"hi"}"#,
        ] {
            assert!(
                serde_json::from_str::<SessionEventsRequest>(hostile).is_err(),
                "hostile body must be rejected: {hostile}",
            );
        }
    }

    #[test]
    fn request_bounds_are_enforced() {
        assert!(
            request(vec![]).validate().is_err(),
            "at least one subscription"
        );
        let too_many: Vec<(String, Option<String>)> = (0..=MAX_ACTIVE_SUBSCRIPTIONS)
            .map(|index| (format!("sub-{index}"), None))
            .collect();
        let too_many = SessionEventsRequest {
            subscriptions: too_many
                .into_iter()
                .map(|(id, after_cursor)| SessionSubscriptionRequest {
                    subscription_id: id,
                    selector: ProductStreamSelector::RunCompletions,
                    after_cursor,
                })
                .collect(),
        };
        assert!(
            too_many.validate().is_err(),
            "at most {MAX_ACTIVE_SUBSCRIPTIONS}"
        );
        assert!(
            request(vec![("a", None), ("a", None)]).validate().is_err(),
            "distinct ids"
        );
        assert!(
            request(vec![("", None)]).validate().is_err(),
            "non-empty id"
        );
        let oversized_id = "a".repeat(MAX_SUBSCRIPTION_ID_BYTES + 1);
        assert!(
            request(vec![(oversized_id.as_str(), None)])
                .validate()
                .is_err()
        );
        let oversized_cursor = "c".repeat(MAX_SUBSCRIBE_CURSOR_BYTES + 1);
        assert!(
            request(vec![("a", Some(oversized_cursor))])
                .validate()
                .is_err()
        );
        assert!(
            request(vec![("a", Some("\"cursor\"".to_string())), ("b", None)])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn server_frames_carry_the_schema_and_travel_under_their_type() {
        let frame = SessionServerFrame::event(
            "chat".to_string(),
            3,
            Some("\"cursor\"".to_string()),
            serde_json::json!({"type": "message"}),
        );
        assert_eq!(frame.name(), "event");
        let json: serde_json::Value = serde_json::to_value(&frame).expect("frame serializes");
        assert_eq!(json["schema"], SESSION_EVENT_SCHEMA);
        assert_eq!(json["type"], "event");
        assert_eq!(json["generation"], 3);
        let hint = SessionServerFrame::lifetime_reconnect_hint();
        assert_eq!(hint.name(), "reconnect_hint");
        let error = SessionServerFrame::subscription_error(
            "chat".to_string(),
            1,
            &ProductSurfaceError::unavailable(true),
            None,
        );
        let json: serde_json::Value = serde_json::to_value(&error).expect("error serializes");
        assert_eq!(json["retryable"], true);
        assert!(
            json.get("detail").is_none(),
            "only the redacted taxonomy travels"
        );
    }
}
