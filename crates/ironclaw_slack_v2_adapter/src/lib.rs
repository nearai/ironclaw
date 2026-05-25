//! Slack v2 ProductAdapter tracer-bullet for Reborn (#3857).
//!
//! This crate owns Slack protocol parsing/rendering only. Hosts verify Slack
//! request signatures, mint `ProtocolAuthEvidence`, stamp trusted inbound
//! context, and route through ProductWorkflow. The adapter never sees raw Slack
//! signing secrets or bot tokens.
//!
//! * [`adapter`] — ProductAdapter implementation and egress/auth metadata.
//! * [`payload`] — Slack Events API payload normalization.
//! * [`render`] — `FinalReplyView` -> `chat.postMessage` request shaping.

pub mod adapter;
pub mod payload;
pub mod render;

pub use adapter::{
    SlackV2Adapter, SlackV2AdapterConfig, slack_declared_egress_hosts, slack_default_capabilities,
    slack_request_signature_auth_requirement,
};
pub use payload::{SLACK_API_HOST, SLACK_USER_ACTOR_KIND, parse_slack_event};
pub use render::{SlackRenderError, SlackReplyTarget, render_final_reply};
