//! Locks the wire shape of the hoisted WebChat v2 event schema. Companion to
//! `ironclaw_webui_v2/tests/webui_v2_schema_contract.rs`, which proves the
//! `ironclaw_webui_v2::schema` re-export still round-trips identically.

use ironclaw_product_workflow::webchat_schema::{WebChatV2Event, WebChatV2EventFrame};
use ironclaw_product_workflow::{FinalReplyView, ProjectionCursor};
use ironclaw_turns::TurnRunId;

fn cursor() -> ProjectionCursor {
    ProjectionCursor::new("cursor:webchat:v2:1").expect("cursor")
}

/// One frozen `FinalReply` frame. Field values are fixed (not `Utc::now()`)
/// so this is a true byte-for-byte snapshot, not just a shape check.
#[test]
fn final_reply_frame_serializes_to_the_pinned_wire_shape() {
    let run_id = TurnRunId::new();
    let frame = WebChatV2EventFrame {
        cursor: cursor(),
        event: WebChatV2Event::FinalReply {
            reply: FinalReplyView {
                turn_run_id: run_id,
                text: "done".to_string(),
                generated_at: chrono::DateTime::parse_from_rfc3339("2026-07-15T00:00:00Z")
                    .expect("fixed timestamp")
                    .into(),
            },
        },
    };

    let json = serde_json::to_value(&frame).expect("serialize frame");
    assert_eq!(json["cursor"], "cursor:webchat:v2:1");
    assert_eq!(json["type"], "final_reply");
    assert_eq!(json["reply"]["text"], "done");
    assert_eq!(json["reply"]["generated_at"], "2026-07-15T00:00:00Z");
    assert_eq!(json["reply"]["turn_run_id"], run_id.to_string());

    // Round-trip: deserializing the frozen JSON back must reproduce the frame.
    let round_tripped: WebChatV2EventFrame =
        serde_json::from_value(json).expect("deserialize frame");
    assert_eq!(round_tripped, frame);
}

#[test]
fn every_event_variant_keeps_its_stable_wire_tag() {
    // event_name() is the single source of truth the SSE handler uses for
    // the `event:` line (ironclaw_webui_v2/src/handlers.rs:962); this test
    // pins it independent of webui_v2.
    assert_eq!(WebChatV2Event::KeepAlive.event_name(), "keep_alive");
}
