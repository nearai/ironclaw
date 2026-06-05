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

#![forbid(unsafe_code)]

mod adapter;
mod payload;
mod render;

pub const SLACK_V2_ADAPTER_ID: &str = "slack_v2";

pub use adapter::{
    SlackV2Adapter, SlackV2AdapterConfig, slack_declared_egress_hosts, slack_default_capabilities,
    slack_request_signature_auth_requirement,
};
pub use payload::{
    SLACK_API_HOST, SLACK_USER_ACTOR_KIND, SlackPayloadParseError, SlackUrlVerificationChallenge,
    parse_slack_event, parse_slack_url_verification_challenge,
};
pub use render::{SlackRenderError, render_final_reply};
