//! Contract tests for the product-tier vocabulary.
//!
//! These two moved verbatim from `ironclaw_host_api`'s `host_api_contract.rs`
//! with the `ProductSurface` membrane they pin; the assertions are unchanged.

use serde_json::json;

use ironclaw_host_api::turn::TurnRunId;
use ironclaw_product_contracts::outbound::{ProductProjectionItem, ProductProjectionState};
use ironclaw_product_contracts::surface::{
    ProductSurfaceEventSubscription, ProductSurfaceStreamResponse,
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
fn product_stream_continuation_is_process_local_not_wire_state() {
    let (_sender, receiver) = tokio::sync::mpsc::channel(1);
    let response = ProductSurfaceStreamResponse {
        events: vec![json!({"kind": "projection_update"})],
        next_cursor: Some("cursor-1".to_string()),
        subscription: Some(ProductSurfaceEventSubscription::new(receiver)),
    };

    let encoded = serde_json::to_value(response).expect("stream response serializes");
    assert_eq!(
        encoded,
        json!({
            "events": [{"kind": "projection_update"}],
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
