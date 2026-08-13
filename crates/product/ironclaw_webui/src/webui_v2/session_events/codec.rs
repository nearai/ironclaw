//! The browser-safe event codec shared by SSE and the session WebSocket.
//!
//! Every typed product stream event crosses toward a browser exactly once,
//! through [`browser_frame`]. The result carries the redacted
//! `WebChatV2EventFrame` schema plus the two transport-neutral presentation
//! decisions both transports must agree on: the stable event name and the
//! resume-cursor token (deliberately absent for keep-alive frames, which are
//! liveness pings rather than durable resume positions).

use ironclaw_product_contracts::surface::ProductStreamEventEnvelope;

use super::super::schema::{WebChatV2Event, WebChatV2EventFrame};

/// One browser-renderable event, decided once for every transport.
pub(crate) struct BrowserFrame {
    /// Stable browser event name (`final_reply`, `projection_update`, …).
    pub(crate) event_name: &'static str,
    /// Resume-cursor token for transports that expose one (`id:` for SSE,
    /// the `cursor` field for session frames). `None` for keep-alives.
    pub(crate) cursor_token: Option<String>,
    /// The redacted frame body; serialize whole for SSE `data:` lines.
    pub(crate) frame: WebChatV2EventFrame,
}

impl BrowserFrame {
    /// The event body without its cursor, for envelope-style transports that
    /// carry the cursor as a sibling field.
    pub(crate) fn event_body(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(&self.frame.event)
    }
}

/// Map one typed product stream event into its browser frame.
pub(crate) fn browser_frame(envelope: ProductStreamEventEnvelope) -> BrowserFrame {
    let frame = WebChatV2EventFrame::from(envelope);
    // Keep-alive frames are liveness pings, not durable resume positions.
    // The product seam stamps an advancing cursor into every envelope
    // (including `KeepAlive`), and browsers echo the last cursor token back
    // on reconnect. If a keep-alive were the last frame before a disconnect,
    // resuming from its cursor would skip real events that precede it, so
    // keep-alives never carry a resume token on any transport.
    let cursor_token = if matches!(&frame.event, WebChatV2Event::KeepAlive) {
        None
    } else {
        cursor_token(&frame)
    };
    BrowserFrame {
        event_name: frame.event_name(),
        cursor_token,
        frame,
    }
}

/// The wire form of a resume cursor: the JSON-serialized projection cursor,
/// accepted back verbatim from `Last-Event-ID`, `?after_cursor=`, and session
/// subscribe frames.
pub(crate) fn cursor_token(frame: &WebChatV2EventFrame) -> Option<String> {
    serde_json::to_string(frame.cursor()).ok()
}
