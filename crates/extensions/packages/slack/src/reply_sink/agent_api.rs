//! The Slack Web API client the reply sink drives over restricted egress,
//! the failure classification every call shares, and the mapping from Slack
//! answers to sink outcomes (design §9: rate limits → `Retryable` with the
//! provider hint, transport ambiguity → `Ambiguous`, `stopped_by_user` →
//! `StoppedByUser`, auth errors → `Unauthorized`, a missing Agent capability
//! → `Permanent` naming it — never a conventional-message fallback).

use std::time::Duration;

use ironclaw_extension_contracts::channel_adapter::PartDeliveryOutcome;
use ironclaw_extension_contracts::reply::{ReplyOutcomeReason, ReplySinkOutcome};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_host_api::{action::NetworkMethod, ids::SecretHandle};
use serde_json::Value;
use url::Url;

use crate::api::SlackWebApiMethod;

// ── Slack Web API client over restricted egress ─────────────────────────

pub(super) struct SlackAgentApi<'a> {
    pub(super) egress: &'a dyn RestrictedEgress,
    pub(super) credential: &'a SecretHandle,
}

/// Why one Slack call did not succeed, before it is mapped to a sink
/// outcome. `Rejected` keeps the error string so call sites can special-case
/// the few errors that change control flow.
#[derive(Debug)]
pub(super) enum SlackApiFailure {
    /// The request crossed into transport and Slack may have applied it:
    /// the transport failed after the send, or a 2xx answer arrived that
    /// this sink cannot read (not JSON, or no boolean `ok`).
    Ambiguous { reason: String },
    /// Slack answered `ok: false`.
    Rejected {
        error: String,
        retry_after: Option<Duration>,
    },
    /// A non-2xx HTTP status.
    Status {
        status: u16,
        retry_after: Option<Duration>,
    },
    /// The host refused the request before the network.
    Egress(RestrictedEgressError),
    /// An `ok: true` answer without the shape the call needs (a read-back
    /// with no `messages` array): the call itself succeeded, its result
    /// proves nothing.
    InvalidResponse { reason: String },
    /// The request could not be built.
    Local { reason: String },
}

impl SlackAgentApi<'_> {
    pub(super) async fn post(
        &self,
        method: SlackWebApiMethod,
        body: Value,
    ) -> Result<Value, SlackApiFailure> {
        let body = serde_json::to_vec(&body).map_err(|error| SlackApiFailure::Local {
            reason: format!("{} body did not serialize: {error}", method.name()),
        })?;
        let response = self
            .egress
            .send(RestrictedEgressRequest {
                method: NetworkMethod::Post,
                url: method.url(),
                headers: vec![(
                    "content-type".to_string(),
                    "application/json; charset=utf-8".to_string(),
                )],
                body: Some(body),
                credential: Some(self.credential.clone()),
                body_credentials: Vec::new(),
            })
            .await;
        classify(method, response)
    }

    pub(super) async fn get(
        &self,
        method: SlackWebApiMethod,
        query: &[(&str, &str)],
    ) -> Result<Value, SlackApiFailure> {
        let mut url = Url::parse(&method.url()).map_err(|error| SlackApiFailure::Local {
            reason: format!("{} URL is invalid: {error}", method.name()),
        })?;
        {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        let response = self
            .egress
            .send(RestrictedEgressRequest {
                method: NetworkMethod::Get,
                url: url.into(),
                headers: Vec::new(),
                body: None,
                credential: Some(self.credential.clone()),
                body_credentials: Vec::new(),
            })
            .await;
        classify(method, response)
    }

    /// The streaming message's current text, when Slack still has it — with
    /// "found but no comparable text" (a message rendered only from blocks
    /// omits `text`) kept distinct from "not found": the first proves
    /// nothing about a pending append, the second proves the message is
    /// gone.
    pub(super) async fn read_back(
        &self,
        channel: &str,
        ts: &str,
    ) -> Result<SlackReadBack, SlackApiFailure> {
        let response = self
            .get(
                SlackWebApiMethod::ConversationsReplies,
                &[
                    ("channel", channel),
                    ("ts", ts),
                    ("limit", "1"),
                    ("inclusive", "true"),
                ],
            )
            .await?;
        // No `messages` array is not "the message is gone" (which would
        // re-send a pending delta): the answer proves nothing.
        let messages = response
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| SlackApiFailure::InvalidResponse {
                reason: "slack conversations.replies answered ok without a messages array"
                    .to_string(),
            })?;
        let message = messages
            .iter()
            .find(|message| message.get("ts").and_then(Value::as_str) == Some(ts));
        Ok(match message {
            None => SlackReadBack::NotFound,
            Some(message) => match message.get("text").and_then(Value::as_str) {
                Some(text) => SlackReadBack::Found(text.to_string()),
                None => SlackReadBack::FoundWithoutText,
            },
        })
    }
}

/// What a `conversations.replies` read-back learned about the streaming
/// message.
#[derive(Debug)]
pub(super) enum SlackReadBack {
    /// The message exists and carries comparable text.
    Found(String),
    /// The message exists but Slack returned no `text` field to compare.
    FoundWithoutText,
    /// The message is gone (or never existed).
    NotFound,
}

fn classify(
    method: SlackWebApiMethod,
    response: Result<RestrictedEgressResponse, RestrictedEgressError>,
) -> Result<Value, SlackApiFailure> {
    let response = match response {
        Ok(response) => response,
        Err(RestrictedEgressError::Transport { reason }) => {
            return Err(SlackApiFailure::Ambiguous {
                reason: format!(
                    "slack {} transport failed after the request was sent: {reason}",
                    method.name()
                ),
            });
        }
        Err(error) => return Err(SlackApiFailure::Egress(error)),
    };
    if !(200..300).contains(&response.status) {
        return Err(SlackApiFailure::Status {
            status: response.status,
            retry_after: response.retry_after,
        });
    }
    // A 2xx crossed transport and was acted on. Only an explicit `ok: false`
    // is a rejection; a body this sink cannot read — not JSON, or `ok`
    // missing or not a boolean — is the lost-answer shape, so the call sites
    // arm the same pending / ghost-stream latches a transport loss arms.
    let value: Value = match serde_json::from_slice(&response.body) {
        Ok(value) => value,
        Err(error) => {
            return Err(SlackApiFailure::Ambiguous {
                reason: format!(
                    "slack {} answered 2xx with a body that is not valid JSON: {error}",
                    method.name()
                ),
            });
        }
    };
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(value),
        Some(false) => Err(SlackApiFailure::Rejected {
            error: value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown_error")
                .to_string(),
            retry_after: response.retry_after,
        }),
        None => Err(SlackApiFailure::Ambiguous {
            reason: format!("slack {} answered 2xx without a boolean ok", method.name()),
        }),
    }
}

// ── Outcome mapping ──────────────────────────────────────────────────────

pub(super) fn outcome_for_failure(
    method: SlackWebApiMethod,
    failure: SlackApiFailure,
) -> ReplySinkOutcome {
    match failure {
        SlackApiFailure::Ambiguous { reason } => {
            // Session status is idempotent: a lost answer is safe to retry.
            if method == SlackWebApiMethod::AgentsSessionsSetStatus {
                ReplySinkOutcome::Retryable {
                    reason: ReplyOutcomeReason::new(reason),
                    retry_after: None,
                }
            } else {
                ReplySinkOutcome::Ambiguous {
                    reason: ReplyOutcomeReason::new(reason),
                }
            }
        }
        SlackApiFailure::Rejected { error, retry_after } => {
            outcome_for_slack_error(method, &error, retry_after)
        }
        SlackApiFailure::Status {
            status,
            retry_after,
        } => {
            let reason = ReplyOutcomeReason::new(format!(
                "slack {} returned status {status}",
                method.name()
            ));
            match status {
                429 => ReplySinkOutcome::Retryable {
                    reason,
                    retry_after,
                },
                408 | 500..=599 => ReplySinkOutcome::Retryable {
                    reason,
                    retry_after: None,
                },
                401 | 403 => ReplySinkOutcome::Unauthorized { reason },
                _ => ReplySinkOutcome::Permanent { reason },
            }
        }
        SlackApiFailure::Egress(error) => {
            let reason = ReplyOutcomeReason::new(error.to_string());
            match error {
                RestrictedEgressError::Transport { .. } => ReplySinkOutcome::Ambiguous { reason },
                RestrictedEgressError::AuthRequired { .. }
                | RestrictedEgressError::UndeclaredCredential { .. } => {
                    ReplySinkOutcome::Unauthorized { reason }
                }
                RestrictedEgressError::UndeclaredHost { .. }
                | RestrictedEgressError::UndeclaredMethod
                | RestrictedEgressError::HostOwnedHeader { .. }
                | RestrictedEgressError::PolicyDenied
                | RestrictedEgressError::ResponseTooLarge => ReplySinkOutcome::Permanent { reason },
            }
        }
        SlackApiFailure::InvalidResponse { reason } => ReplySinkOutcome::Ambiguous {
            reason: ReplyOutcomeReason::new(reason),
        },
        SlackApiFailure::Local { reason } => ReplySinkOutcome::Permanent {
            reason: ReplyOutcomeReason::new(reason),
        },
    }
}

pub(super) fn outcome_for_slack_error(
    method: SlackWebApiMethod,
    error: &str,
    retry_after: Option<Duration>,
) -> ReplySinkOutcome {
    let name = method.name();
    let reason = ReplyOutcomeReason::new(format!("slack rejected {name} ({error})"));
    match error {
        "ratelimited" | "rate_limited" => ReplySinkOutcome::Retryable {
            reason,
            retry_after,
        },
        "internal_error" | "service_unavailable" | "request_timeout" | "fatal_error" => {
            ReplySinkOutcome::Retryable {
                reason,
                retry_after: None,
            }
        }
        "stopped_by_user" => ReplySinkOutcome::StoppedByUser,
        "invalid_auth" | "not_authed" | "token_revoked" | "token_expired" | "account_inactive" => {
            ReplySinkOutcome::Unauthorized { reason }
        }
        "feature_disabled" | "not_agent_app" => ReplySinkOutcome::Permanent {
            reason: ReplyOutcomeReason::new(format!(
                "slack rejected {name} ({error}): this Slack app is not an Agent — enable the \
                 Agents feature (`features.agent_view` in the app manifest, which adds the \
                 assistant:write scope) and reinstall the app; there is no conventional-message \
                 fallback"
            )),
        },
        "missing_scope" => ReplySinkOutcome::Permanent {
            reason: ReplyOutcomeReason::new(format!(
                "slack rejected {name} (missing_scope): the bot token lacks a scope the native \
                 Agent surface needs (chat:write for {name}; assistant:write arrives with the \
                 Agents feature) — update the app's bot scopes and reinstall it"
            )),
        },
        // message_not_in_streaming_state, message_not_owned_by_app,
        // streaming_mode_mismatch, channel_not_found, is_archived,
        // not_authorized, invalid_arguments, ...
        _ => ReplySinkOutcome::Permanent { reason },
    }
}

pub(super) fn outcome_for_part(outcome: PartDeliveryOutcome) -> ReplySinkOutcome {
    match outcome {
        PartDeliveryOutcome::Sent { .. } => ReplySinkOutcome::Applied,
        PartDeliveryOutcome::Retryable { reason } => ReplySinkOutcome::Retryable {
            reason: ReplyOutcomeReason::new(reason),
            retry_after: None,
        },
        PartDeliveryOutcome::Ambiguous { reason } => ReplySinkOutcome::Ambiguous {
            reason: ReplyOutcomeReason::new(reason),
        },
        PartDeliveryOutcome::Permanent { reason } => ReplySinkOutcome::Permanent {
            reason: ReplyOutcomeReason::new(reason),
        },
        PartDeliveryOutcome::Unauthorized { reason } => ReplySinkOutcome::Unauthorized {
            reason: ReplyOutcomeReason::new(reason),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered_2xx(body: &[u8]) -> Result<Value, SlackApiFailure> {
        classify(
            SlackWebApiMethod::ChatAppendStream,
            Ok(RestrictedEgressResponse {
                status: 200,
                body: body.to_vec(),
                retry_after: None,
            }),
        )
    }

    /// A 2xx answer crossed transport and was acted on: only an explicit
    /// `ok: false` is a rejection; a body that cannot be read (not JSON, or
    /// `ok` missing or not a boolean) is the lost-answer shape.
    #[test]
    fn a_2xx_answer_is_ambiguous_unless_ok_is_a_boolean() {
        assert!(answered_2xx(br#"{"ok":true,"ts":"1710000100.000001"}"#).is_ok());
        assert!(matches!(
            answered_2xx(br#"{"ok":false,"error":"invalid_arguments"}"#),
            Err(SlackApiFailure::Rejected { error, .. }) if error == "invalid_arguments"
        ));
        for body in [
            &b"<html><body>upstream error</body></html>"[..],
            b"{}",
            br#"{"ok":"yes"}"#,
            br#"{"ok":1}"#,
        ] {
            assert!(
                matches!(answered_2xx(body), Err(SlackApiFailure::Ambiguous { .. })),
                "{} must be ambiguous",
                String::from_utf8_lossy(body)
            );
        }
    }
}
