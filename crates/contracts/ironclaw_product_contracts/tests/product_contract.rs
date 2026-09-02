//! Contract tests for the product-tier vocabulary.
//!
//! These two moved verbatim from `ironclaw_host_api`'s `host_api_contract.rs`
//! with the `ProductSurface` membrane they pin; the assertions are unchanged.

use serde_json::json;

use ironclaw_host_api::turn::TurnRunId;
use ironclaw_product_contracts::outbound::{
    ProductOutboundPayload, ProductProjectionItem, ProductProjectionState, ProjectionCursor,
};
use ironclaw_product_contracts::surface::{
    ProductStreamEvent, ProductStreamEventEnvelope, ProductStreamSelector,
    ProductSurfaceEventSubscription, ProductSurfaceStreamRequest, ProductSurfaceStreamResponse,
};

#[test]
fn projection_text_distinguishes_live_from_finalized_transcript_rows() {
    let run_id = TurnRunId::new();
    let decoded: ProductProjectionState = serde_json::from_value(json!({
        "thread_id": "thread-1",
        "items": [{"text": {"id": "live-1", "run_id": run_id, "body": "partial"}}]
    }))
    .expect("legacy live text remains readable");
    assert!(matches!(
        decoded.items.as_slice(),
        [ProductProjectionItem::Text {
            finalized: false,
            ..
        }]
    ));

    let finalized = ProductProjectionState::new(
        "thread-1",
        vec![ProductProjectionItem::Text {
            id: "message-1".to_string(),
            run_id: Some(run_id),
            body: "final".to_string(),
            finalized: true,
        }],
    )
    .expect("finalized text state");
    let value = serde_json::to_value(&finalized).expect("serialize finalized text");
    assert_eq!(value["items"][0]["text"]["finalized"], true);
    assert_eq!(
        serde_json::from_value::<ProductProjectionState>(value).expect("round trip finalized text"),
        finalized
    );
}

#[test]
fn product_stream_selector_wire_shape_is_typed_and_tagged() {
    let request = ProductSurfaceStreamRequest {
        selector: ProductStreamSelector::Thread {
            thread_id: "thread-1".to_string(),
        },
        after_cursor: Some("cursor-1".to_string()),
    };
    let encoded = serde_json::to_value(&request).expect("stream request serializes");
    assert_eq!(
        encoded,
        json!({
            "selector": {"kind": "thread", "thread_id": "thread-1"},
            "after_cursor": "cursor-1"
        }),
        "the selector is a closed tagged vocabulary, not a magic string"
    );
    let decoded: ProductSurfaceStreamRequest =
        serde_json::from_value(encoded).expect("stream request deserializes");
    assert_eq!(decoded, request);
}

#[test]
fn product_stream_events_stay_typed_through_the_response_envelope() {
    let cursor = ProjectionCursor::new("cursor-1").expect("bounded cursor");
    let envelope = ProductStreamEventEnvelope {
        cursor: cursor.clone(),
        event: ProductStreamEvent::Thread(ProductOutboundPayload::KeepAlive),
    };
    let encoded = serde_json::to_value(&envelope).expect("stream event envelope serializes");
    assert_eq!(
        encoded,
        json!({
            "cursor": "cursor-1",
            "event": {"kind": "thread", "payload": "keep_alive"}
        }),
        "each event carries its own cursor and a kind-tagged typed payload"
    );
    let decoded: ProductStreamEventEnvelope =
        serde_json::from_value(encoded).expect("stream event envelope deserializes");
    assert_eq!(decoded, envelope);
}

#[test]
fn product_stream_continuation_is_process_local_not_wire_state() {
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let cursor = ProjectionCursor::new("cursor-1").expect("bounded cursor");
    let response = ProductSurfaceStreamResponse {
        events: vec![ProductStreamEventEnvelope {
            cursor,
            event: ProductStreamEvent::Thread(ProductOutboundPayload::KeepAlive),
        }],
        next_cursor: Some("cursor-1".to_string()),
        subscription: Some(ProductSurfaceEventSubscription::new(receiver)),
    };

    let encoded = serde_json::to_value(response).expect("stream response serializes");
    assert_eq!(
        encoded,
        json!({
            "events": [{
                "cursor": "cursor-1",
                "event": {"kind": "thread", "payload": "keep_alive"}
            }],
            "next_cursor": "cursor-1"
        }),
        "the in-process continuation handle must never cross the wire"
    );
    let decoded: ProductSurfaceStreamResponse =
        serde_json::from_value(encoded).expect("wire response deserializes");
    assert!(decoded.subscription.is_none());
}

#[test]
fn product_stream_continuation_debug_and_identity_are_stable() {
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let subscription = ProductSurfaceEventSubscription::new(receiver);
    let (_other_sender, other_receiver) = tokio::sync::mpsc::channel(1);
    let other_subscription = ProductSurfaceEventSubscription::new(other_receiver);

    assert_eq!(subscription, subscription);
    assert_ne!(subscription, other_subscription);
    assert_eq!(
        format!("{subscription:?}"),
        "ProductSurfaceEventSubscription { .. }"
    );
}

#[test]
fn run_completions_selector_and_event_wire_tags_are_pinned() {
    // The browser codec and session client match on these exact strings; a
    // silent rename would strand every subscription.
    let selector = ironclaw_product_contracts::surface::ProductStreamSelector::RunCompletions;
    assert_eq!(
        serde_json::to_value(&selector).expect("selector serializes"),
        serde_json::json!({"kind": "run_completions"}),
    );
    let decoded: ironclaw_product_contracts::surface::ProductStreamSelector =
        serde_json::from_value(serde_json::json!({"kind": "run_completions"}))
            .expect("selector deserializes");
    assert_eq!(decoded, selector);

    let event = ironclaw_product_contracts::surface::ProductStreamEvent::RunCompletion(
        ironclaw_product_contracts::run_completions::RunCompletionStreamEvent::Clear(
            ironclaw_product_contracts::run_completions::RunCompletionClearEvent {
                schema: ironclaw_product_contracts::run_completions::RUN_COMPLETION_CLEAR_SCHEMA
                    .to_string(),
                sequence: "7".to_string(),
                notice_id: "rcn-a".to_string(),
                thread_id: "thread-a".to_string(),
                thread_tag: "rct-a".to_string(),
                read_at: "2026-08-31T00:00:00Z".to_string(),
            },
        ),
    );
    let encoded = serde_json::to_value(&event).expect("event serializes");
    assert_eq!(encoded["kind"], "run_completion");
    assert_eq!(encoded["payload"]["type"], "clear");
    let round: ironclaw_product_contracts::surface::ProductStreamEvent =
        serde_json::from_value(encoded).expect("event deserializes");
    assert_eq!(round, event);
}
