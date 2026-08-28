//! Slack channel extension for Reborn (#3857).
//!
//! This crate owns Slack protocol parsing/rendering only. Hosts verify Slack
//! request signatures, stamp trusted inbound context, and route the translated
//! message through the host-owned channel workflow. The package sees only
//! restricted egress handles, never raw Slack signing secrets or bot tokens.
//!
//! * [`channel`] — `ChannelIngress`/`ChannelReply`/`ChannelDelivery`: complete
//!   inbound translation (attachments + shared context) and Slack-native
//!   output through restricted egress.
//! * [`delivery`] — Slack Web API response classification and status mapping.
//! * [`mrkdwn`] — Slack mrkdwn rendering and message chunking.
//! * [`payload`] — Slack Events API payload normalization.
//! * [`preference_targets`] — reply-target binding-ref grammar + the
//!   preference-target codec for the generic triggered-delivery driver.

#![forbid(unsafe_code)]

mod attachment_transfer;
mod channel;
mod conversation_context;
mod delivery;
mod mrkdwn;
mod payload;
mod preference_targets;

pub const SLACK_V2_ADAPTER_ID: &str = "slack_v2";

pub use channel::SlackChannelAdapter;
pub use payload::{
    SLACK_API_HOST, SLACK_USER_ACTOR_KIND, SlackIgnoreReason, SlackInboundEvent,
    SlackPayloadParseError, normalize_slack_event,
};
pub use preference_targets::{
    SlackPreferenceTargetCodec, SlackReplyTargetError,
    slack_conversation_id_from_reply_target_binding_ref,
    slack_personal_dm_reply_target_binding_ref, slack_reply_target_binding_ref_from_raw,
    slack_reply_target_is_personal_dm, slack_shared_channel_reply_target_binding_ref,
};
