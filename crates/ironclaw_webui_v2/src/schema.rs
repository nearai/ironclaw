//! Re-export of the browser-visible WebChat v2 event schema.
//!
//! The schema itself lives in `ironclaw_product_workflow::webchat_schema`
//! (hoisted 2026-07) so non-route consumers, notably the Reborn TUI client,
//! can depend on the wire contract without depending on this route/handler
//! crate. Handlers in this crate keep using `crate::schema::WebChatV2EventFrame`
//! unchanged.

pub use ironclaw_product_workflow::webchat_schema::{WebChatV2Event, WebChatV2EventFrame};
