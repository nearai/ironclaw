//! Slack Web API delivery-status classification.
//!
//! The `chat.postMessage` egress path itself lives on the live
//! `SlackChannelAdapter` (`channel.rs`); this module classifies the vendor's
//! HTTP status and Slack error codes into a retry/authorization/permanent
//! failure kind for that path. (The retired `ProductAdapter` render/delivery
//! helpers were removed in P7b DEL-5.)

pub(crate) enum SlackDeliveryFailureKind {
    Unauthorized,
    Retryable,
    Permanent,
}

impl SlackDeliveryFailureKind {
    pub(crate) fn from_http_status(status: u16) -> Self {
        if status >= 500 || status == 429 || status == 408 {
            Self::Retryable
        } else if status == 401 || status == 403 {
            Self::Unauthorized
        } else {
            Self::Permanent
        }
    }
}

pub(crate) fn slack_error_kind(error: &str) -> SlackDeliveryFailureKind {
    match error {
        "not_authed"
        | "invalid_auth"
        | "account_inactive"
        | "token_revoked"
        | "missing_scope"
        | "no_permission"
        | "is_bot"
        | "not_allowed_token_type" => SlackDeliveryFailureKind::Unauthorized,
        "fatal_error"
        | "internal_error"
        | "service_unavailable"
        | "request_timeout"
        | "ratelimited" => SlackDeliveryFailureKind::Retryable,
        _ => SlackDeliveryFailureKind::Permanent,
    }
}
