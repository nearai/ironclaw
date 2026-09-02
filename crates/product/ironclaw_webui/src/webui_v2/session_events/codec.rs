//! The browser-safe event codec shared by SSE and the session WebSocket.
//!
//! Every typed product stream event crosses toward a browser exactly once,
//! through [`browser_frame`]. The result carries the redacted
//! `WebChatV2EventFrame` schema plus the two transport-neutral presentation
//! decisions both transports must agree on: the stable event name and the
//! resume-cursor token (deliberately absent for keep-alive frames, which are
//! liveness pings rather than durable resume positions).

use ironclaw_product_contracts::surface::{ProductStreamEvent, ProductStreamEventEnvelope};

use crate::webui_v2::schema::{WebChatV2Event, WebChatV2EventFrame};

/// One browser-renderable event, decided once for every transport.
pub(crate) struct BrowserFrame {
    /// Stable browser event name (`final_reply`, `run_completion`, …).
    pub(crate) event_name: &'static str,
    /// Resume-cursor token for transports that expose one (`id:` for SSE,
    /// the `cursor` field for session frames). `None` for keep-alives.
    pub(crate) cursor_token: Option<String>,
    /// The full redacted frame (cursor + tagged event) for SSE `data:`
    /// lines.
    frame_body: serde_json::Value,
    /// The tagged event alone, for envelope transports that carry the
    /// cursor as a sibling field.
    event_body: serde_json::Value,
}

impl BrowserFrame {
    pub(crate) fn sse_data(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.frame_body)
    }

    /// The typed event body already materialized by [`browser_frame`]; no
    /// serialization happens here.
    pub(crate) fn event_body(&self) -> serde_json::Value {
        self.event_body.clone()
    }
}

/// Map one typed product stream event into its browser frame. `None` means
/// the event could not be rendered; the transports fail the stream or
/// subscription loudly so the client resumes from its last cursor and the
/// event is never silently skipped.
pub(crate) fn browser_frame(envelope: ProductStreamEventEnvelope) -> Option<BrowserFrame> {
    let ProductStreamEventEnvelope { cursor, event } = envelope;
    match event {
        ProductStreamEvent::Thread(payload) => {
            let frame = WebChatV2EventFrame {
                cursor,
                event: WebChatV2Event::from(payload),
            };
            // Keep-alive frames are liveness pings, not durable resume
            // positions. The product seam stamps an advancing cursor into
            // every envelope (including `KeepAlive`), and browsers echo the
            // last cursor token back on reconnect. If a keep-alive were the
            // last frame before a disconnect, resuming from its cursor
            // would skip real events that precede it, so keep-alives never
            // carry a resume token on any transport.
            let cursor_token = if matches!(&frame.event, WebChatV2Event::KeepAlive) {
                None
            } else {
                serde_json::to_string(frame.cursor()).ok()
            };
            Some(BrowserFrame {
                event_name: frame.event_name(),
                cursor_token,
                event_body: serde_json::to_value(&frame.event).ok()?,
                frame_body: serde_json::to_value(&frame).ok()?,
            })
        }
        ProductStreamEvent::RunCompletion(event) => {
            let cursor_token = serde_json::to_string(&cursor).ok();
            let event_body = serde_json::to_value(&event).ok()?;
            let frame_body = serde_json::json!({
                "cursor": cursor,
                "type": "run_completion",
                "event": event_body,
            });
            Some(BrowserFrame {
                event_name: "run_completion",
                cursor_token,
                event_body: serde_json::json!({
                    "type": "run_completion",
                    "event": event_body,
                }),
                frame_body,
            })
        }
    }
}
