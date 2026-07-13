//! Slack v2 ProductAdapter tracer-bullet for Reborn (#3857).
//!
//! This crate owns Slack protocol parsing/rendering only. Hosts verify Slack
//! request signatures, mint `ProtocolAuthEvidence`, stamp trusted inbound
//! context, and route through ProductWorkflow. The adapter never sees raw Slack
//! signing secrets or bot tokens.
//!
//! * [`adapter`] — ProductAdapter implementation and egress/auth metadata.
//! * [`channel`] — the generic-ingress `ChannelAdapter` (inbound cutover).
//! * [`delivery`] — Slack Web API response classification and status mapping.
//! * [`mrkdwn`] — Slack mrkdwn rendering and message chunking.
//! * [`payload`] — Slack Events API payload normalization.
//! * [`preference_targets`] — reply-target binding-ref grammar + the
//!   preference-target codec for the generic triggered-delivery driver.
//! * [`render`] — `FinalReplyView` -> `chat.postMessage` request shaping.

#![forbid(unsafe_code)]

mod adapter;
mod channel;
mod delivery;
mod mrkdwn;
mod payload;
mod preference_targets;
mod render;

pub const SLACK_V2_ADAPTER_ID: &str = "slack_v2";

pub use adapter::{
    SlackV2Adapter, SlackV2AdapterConfig, slack_declared_egress_hosts, slack_default_capabilities,
    slack_request_signature_auth_requirement,
};
pub use channel::SlackChannelAdapter;
pub use payload::{
    SLACK_API_HOST, SLACK_USER_ACTOR_KIND, SlackInboundEvent, SlackNormalizedMessage,
    SlackPayloadParseError, SlackUrlVerificationChallenge, classify_interaction_resolution,
    normalize_slack_event, parse_slack_event, parse_slack_url_verification_challenge,
};
pub use preference_targets::{
    SlackPreferenceTargetCodec, SlackReplyTargetError,
    slack_conversation_id_from_reply_target_binding_ref,
    slack_personal_dm_reply_target_binding_ref, slack_reply_target_binding_ref_from_raw,
    slack_reply_target_is_personal_dm, slack_shared_channel_reply_target_binding_ref,
};
pub use render::{SlackRenderError, render_final_reply};
