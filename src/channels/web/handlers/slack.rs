//! Slack core HTTP handlers (slash commands, interactivity).
//!
//! Distinct from the Slack WASM channel under `channels-src/slack/`,
//! which handles inbound *Events API* webhooks at `/webhook/slack`.
//! Slash commands and interactivity have a different contract — Slack
//! requires a synchronous ack within 3 seconds and the actual reply
//! flows out-of-band via a one-shot `response_url` — so they're handled
//! by core IronClaw routes registered here.
//!
//! The signature verifier and body parser live in `crate::channels::slack`
//! as pure modules; this file just plumbs axum extractors through them.
//!
//! ## What this commit ships
//!
//! `slash_command_handler` —
//! 1. Pulls `slack_signing_secret` from the secrets store, keyed on the
//!    deployment owner (matches the WASM channel's secret name).
//! 2. Verifies `X-Slack-Signature` against the raw bytes via
//!    `crate::channels::slack::sig::verify` (constant-time-compared).
//! 3. Parses the form-encoded body via `parse_slash_command`.
//! 4. Confirms the workspace is installed by resolving
//!    `(channel='slack', external_id=workspace_id)` in
//!    `channel_identities` — un-installed workspaces get a friendly
//!    "this workspace isn't bound to an IronClaw deployment" reply.
//! 5. Acks ephemerally within 3 seconds.
//!
//! ## What's deliberately NOT here yet
//!
//! * Agent invocation + the out-of-band POST to `response_url` —
//!   threading the engine through this path needs care; lands in a
//!   subsequent commit.
//! * Per-user pairing on first invocation from an unknown user.
//! * Audit-log row for in/out compliance trail (the `channel_audit_log`
//!   table itself lands in a follow-up commit).
//! * Interactivity surface (`/api/channels/slack/interactivity`) — same
//!   verifier, different body shape; one commit at a time.

use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::channels::slack::{
    SLACK_SIGNING_SECRET, SignatureError, SlashParseError, VerifyInputs, ack_payload,
    effective_workspace_id, parse_slash_command, verify,
};
use crate::channels::web::platform::state::GatewayState;

/// POST `/api/channels/slack/slash` — receives every `/ironclaw <…>`
/// invocation across every workspace where the IronClaw Slack app is
/// installed.
pub async fn slash_command_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let now_secs = chrono::Utc::now().timestamp();

    // 1. Pull signing secret. If unset, the gateway is misconfigured —
    //    surface a 503 so the operator notices, not 500.
    let signing_secret = match read_signing_secret(&state).await {
        Ok(s) => s,
        Err(reason) => {
            tracing::warn!(
                reason,
                "slash command rejected: signing secret not available"
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Slack channel is not configured on this deployment.",
            )
                .into_response();
        }
    };

    // 2. Verify signature against raw bytes (Slack signs the wire bytes,
    //    NOT any decoded form).
    let timestamp_header = header_str(&headers, "x-slack-request-timestamp");
    let signature_header = header_str(&headers, "x-slack-signature");
    if let Err(err) = verify(VerifyInputs {
        timestamp_header,
        signature_header,
        body: body.as_ref(),
        signing_secret: signing_secret.as_bytes(),
        now_secs,
    }) {
        return signature_error_response(err);
    }

    // 3. Parse the form-encoded body.
    let req = match parse_slash_command(body.as_ref()) {
        Ok(r) => r,
        Err(err) => return parse_error_response(err),
    };

    // 4. Resolve workspace identity. Lack of a row means the workspace
    //    never ran `ironclaw channels install slack <team>` — friendly
    //    self-service hint rather than a vague 4xx.
    let store = match state.store.as_ref() {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "IronClaw database is not available.",
            )
                .into_response();
        }
    };
    let workspace_id = effective_workspace_id(&req);
    let resolved = match store.resolve_channel_identity("slack", workspace_id).await {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!(error = %e, "resolve_channel_identity failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ack_payload(
                    "IronClaw hit a database error resolving this workspace. Please try again.",
                )),
            )
                .into_response();
        }
    };
    if resolved.is_none() {
        // Slack wants 200 with `response_type=ephemeral` here so the user
        // sees the message instead of a generic Slack error.
        return (
            StatusCode::OK,
            Json(ack_payload(&format!(
                "This Slack workspace ({workspace_id}) is not bound to an IronClaw deployment yet. \
                 Run `ironclaw channels install slack {workspace_id}` against the IronClaw \
                 instance you want to bind to this workspace."
            ))),
        )
            .into_response();
    }

    // 5. Ack within 3s. The actual agent reply lands later via response_url
    //    (next commit on this branch).
    let placeholder = if req.text.trim().is_empty() {
        "On it. (Empty prompt — IronClaw will ignore this invocation.)".to_string()
    } else {
        format!("On it: \"{}\"", truncate_for_display(&req.text, 120))
    };
    (StatusCode::OK, Json(ack_payload(&placeholder))).into_response()
}

async fn read_signing_secret(state: &GatewayState) -> Result<String, &'static str> {
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or("secrets_store missing")?;
    let decrypted = secrets
        .get_decrypted(&state.owner_id, SLACK_SIGNING_SECRET)
        .await
        .map_err(|_| "slack_signing_secret not present")?;
    Ok(decrypted.expose().to_string())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

fn signature_error_response(err: SignatureError) -> Response {
    // Distinguish replay / mismatch (401) from operator-controllable
    // misconfiguration so logs are useful but the wire stays terse.
    let status = match err {
        SignatureError::Stale => StatusCode::REQUEST_TIMEOUT,
        SignatureError::MissingTimestamp
        | SignatureError::MissingSignature
        | SignatureError::InvalidTimestamp(_)
        | SignatureError::BadSignaturePrefix
        | SignatureError::BadSignatureHex(_) => StatusCode::BAD_REQUEST,
        SignatureError::Mismatch => StatusCode::UNAUTHORIZED,
    };
    tracing::warn!(error = %err, "slash command signature check failed");
    (status, "Slack signature check failed").into_response()
}

fn parse_error_response(err: SlashParseError) -> Response {
    let status = match err {
        SlashParseError::NotUtf8(_) => StatusCode::BAD_REQUEST,
        SlashParseError::MissingField(_) => StatusCode::UNPROCESSABLE_ENTITY,
    };
    tracing::warn!(error = %err, "slash command body invalid");
    (status, "Slack slash-command body could not be parsed").into_response()
}

/// Truncate a string at `max_chars` *characters* (not bytes). Used only
/// for placeholder text echoed back to the user; not security-critical.
fn truncate_for_display(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars));
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_str_returns_empty_for_missing_header() {
        let h = HeaderMap::new();
        assert_eq!(header_str(&h, "x-slack-signature"), "");
    }

    #[test]
    fn header_str_returns_value_for_present_header() {
        let mut h = HeaderMap::new();
        h.insert("x-slack-signature", "v0=abcd".parse().unwrap());
        assert_eq!(header_str(&h, "x-slack-signature"), "v0=abcd");
    }

    #[test]
    fn truncate_for_display_appends_ellipsis_above_limit() {
        assert_eq!(truncate_for_display("abcdef", 3), "abc…");
        assert_eq!(truncate_for_display("abc", 3), "abc");
        assert_eq!(truncate_for_display("ab", 3), "ab");
    }

    #[test]
    fn truncate_handles_multibyte_chars_without_panic() {
        // 5 multibyte chars; truncation at 3 must not panic on byte boundaries.
        let s = "αβγδε";
        let out = truncate_for_display(s, 3);
        assert_eq!(out, "αβγ…");
    }
}
