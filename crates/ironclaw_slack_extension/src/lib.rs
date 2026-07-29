//! Slack channel extension for Reborn (#3857).
//!
//! This crate owns Slack protocol parsing/rendering only. Hosts verify Slack
//! request signatures, stamp trusted inbound context, and route through
//! ProductSurface admission. The adapter never sees raw Slack signing secrets or bot
//! tokens.
//!
//! * [`channel`] — the generic-ingress `ChannelAdapter` (live inbound/outbound,
//!   incl. `chat.postMessage` egress + mrkdwn rendering).
//! * [`delivery`] — Slack Web API response classification and status mapping.
//! * [`mrkdwn`] — Slack mrkdwn rendering and message chunking.
//! * [`payload`] — Slack Events API payload normalization.
//! * [`preference_targets`] — reply-target binding-ref grammar + the
//!   preference-target codec for the generic triggered-delivery driver.

#![forbid(unsafe_code)]

mod channel;
mod delivery;
mod mrkdwn;
mod payload;
mod preference_targets;

pub const SLACK_V2_ADAPTER_ID: &str = "slack_v2";

pub use channel::SlackChannelAdapter;
pub use payload::{
    SLACK_API_HOST, SLACK_USER_ACTOR_KIND, SlackInboundEvent, SlackPayloadParseError,
    SlackUrlVerificationChallenge, classify_channel_interaction_resolution,
    classify_interaction_resolution, normalize_slack_event, parse_slack_event,
    parse_slack_url_verification_challenge,
};
pub use preference_targets::{
    SlackPreferenceTargetCodec, SlackReplyTargetError,
    slack_conversation_id_from_reply_target_binding_ref,
    slack_personal_dm_reply_target_binding_ref, slack_reply_target_binding_ref_from_raw,
    slack_reply_target_is_personal_dm, slack_shared_channel_reply_target_binding_ref,
};
