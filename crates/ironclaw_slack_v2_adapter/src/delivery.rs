//! Slack Web API delivery and status classification.

use ironclaw_product_adapters::redaction::RedactedString;
use ironclaw_product_adapters::{
    DeliveryAttemptId, DeliveryStatus, EgressRequest, ProductAdapterError, ProtocolHttpEgress,
    ProtocolHttpEgressError,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde::Deserialize;

/// Maximum accepted byte length for a Slack `chat.postMessage` response body.
/// Protects against WAF/proxy responses (e.g. large HTML error pages on 200 OK)
/// causing full-allocation and O(n) deserialization on the delivery hot path.
const MAX_SLACK_RESPONSE_BYTES: usize = 64 * 1024; // 64 KB

pub(crate) struct SlackPostMessageDeliveryError {
    pub(crate) status: DeliveryStatus,
    pub(crate) adapter_error: ProductAdapterError,
}

pub(crate) async fn send_slack_post_message(
    egress: &dyn ProtocolHttpEgress,
    request: EgressRequest,
    attempt_id: DeliveryAttemptId,
    target_binding: &ReplyTargetBindingRef,
    run_id: Option<TurnRunId>,
) -> Result<(), SlackPostMessageDeliveryError> {
    let response = match egress.send(request).await {
        Ok(response) => response,
        Err(egress_err) => {
            let failure = SlackDeliveryFailureKind::from_egress_error(&egress_err);
            let reason = RedactedString::new(egress_err.to_string());
            return Err(slack_delivery_error(
                failure,
                attempt_id,
                target_binding,
                run_id,
                reason,
            ));
        }
    };

    if !(200..300).contains(&response.status()) {
        let reason = RedactedString::new(format!(
            "slack web api returned status {}",
            response.status()
        ));
        let failure = SlackDeliveryFailureKind::from_http_status(response.status());
        return Err(slack_delivery_error(
            failure,
            attempt_id,
            target_binding,
            run_id,
            reason,
        ));
    }

    if let Err(slack_err) = slack_post_message_result(response.body()) {
        return Err(slack_delivery_error(
            slack_err.kind,
            attempt_id,
            target_binding,
            run_id,
            slack_err.reason,
        ));
    }

    Ok(())
}

fn slack_delivery_error(
    failure: SlackDeliveryFailureKind,
    attempt_id: DeliveryAttemptId,
    target_binding: &ReplyTargetBindingRef,
    run_id: Option<TurnRunId>,
    reason: RedactedString,
) -> SlackPostMessageDeliveryError {
    let status = match failure {
        SlackDeliveryFailureKind::Retryable => DeliveryStatus::FailedRetryable {
            attempt_id,
            target: target_binding.clone(),
            run_id,
            reason: reason.clone(),
        },
        SlackDeliveryFailureKind::Unauthorized => DeliveryStatus::FailedUnauthorized {
            attempt_id,
            target: target_binding.clone(),
            run_id,
            reason: reason.clone(),
        },
        SlackDeliveryFailureKind::Permanent => DeliveryStatus::FailedPermanent {
            attempt_id,
            target: target_binding.clone(),
            run_id,
            reason: reason.clone(),
        },
    };
    SlackPostMessageDeliveryError {
        status,
        adapter_error: failure.to_adapter_error(reason),
    }
}

fn slack_post_message_result(body: &[u8]) -> Result<(), SlackPostMessageFailure> {
    if body.len() > MAX_SLACK_RESPONSE_BYTES {
        return Err(SlackPostMessageFailure::permanent(
            "response body too large",
        ));
    }
    let parsed: SlackPostMessageResponse = serde_json::from_slice(body).map_err(|err| {
        // A truncated/empty body from a proxy/LB timeout is a transient infra
        // condition; treat as retryable rather than permanently abandoning.
        SlackPostMessageFailure {
            reason: RedactedString::new(format!(
                "Slack chat.postMessage response was not valid JSON: {err}"
            )),
            kind: SlackDeliveryFailureKind::Retryable,
        }
    })?;
    if parsed.ok {
        Ok(())
    } else {
        let error = parsed.error.unwrap_or_else(|| "unknown_error".to_string());
        Err(SlackPostMessageFailure {
            reason: RedactedString::new(format!("Slack rejected chat.postMessage ({})", error)),
            kind: slack_error_kind(&error),
        })
    }
}

#[derive(Debug, Deserialize)]
struct SlackPostMessageResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlackDeliveryFailureKind {
    Unauthorized,
    Retryable,
    Permanent,
}

impl SlackDeliveryFailureKind {
    fn from_egress_error(err: &ProtocolHttpEgressError) -> Self {
        match err {
            ProtocolHttpEgressError::Timeout
            | ProtocolHttpEgressError::Network(_)
            | ProtocolHttpEgressError::LeakDetected => Self::Retryable,
            ProtocolHttpEgressError::UnknownCredentialHandle { .. }
            | ProtocolHttpEgressError::UnauthorizedCredentialHandle { .. } => Self::Unauthorized,
            ProtocolHttpEgressError::UndeclaredHost { .. }
            | ProtocolHttpEgressError::PolicyDenied { .. } => Self::Permanent,
        }
    }

    fn from_http_status(status: u16) -> Self {
        if status >= 500 || status == 429 || status == 408 {
            Self::Retryable
        } else if status == 401 || status == 403 {
            Self::Unauthorized
        } else {
            Self::Permanent
        }
    }

    fn to_adapter_error(self, reason: RedactedString) -> ProductAdapterError {
        match self {
            Self::Retryable => ProductAdapterError::EgressTransient { reason },
            Self::Unauthorized | Self::Permanent => ProductAdapterError::EgressDenied { reason },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlackPostMessageFailure {
    reason: RedactedString,
    kind: SlackDeliveryFailureKind,
}

impl SlackPostMessageFailure {
    fn permanent(reason: impl Into<String>) -> Self {
        Self {
            reason: RedactedString::new(reason.into()),
            kind: SlackDeliveryFailureKind::Permanent,
        }
    }
}

fn slack_error_kind(error: &str) -> SlackDeliveryFailureKind {
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
