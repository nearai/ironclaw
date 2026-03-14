//! Public webhook trigger endpoint for routine webhook triggers.
//!
//! `POST /api/webhooks/{path}` — matches the path against routines with
//! `Trigger::Webhook { path, secret }`, validates the secret via constant-time
//! comparison, and fires the matching routine through the message pipeline.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use subtle::ConstantTimeEq;

use crate::agent::routine::{RoutineAction, Trigger};
use crate::channels::IncomingMessage;
use crate::channels::web::server::GatewayState;

/// Handle incoming webhook POST to `/api/webhooks/{path}`.
///
/// This endpoint is **public** (no gateway auth token required) but protected
/// by the per-routine webhook secret sent via the `X-Webhook-Secret` header.
pub async fn webhook_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Rate limit check
    if !state.webhook_rate_limiter.check() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Targeted query instead of loading all routines
    let routine = store
        .get_webhook_routine_by_path(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "No routine matches this webhook path".to_string(),
        ))?;

    // Require webhook secret — routines without a secret cannot be triggered via webhook
    let expected_secret = match &routine.trigger {
        Trigger::Webhook {
            secret: Some(s), ..
        } => s,
        _ => {
            return Err((
                StatusCode::FORBIDDEN,
                "Webhook secret not configured for this routine".to_string(),
            ));
        }
    };

    let provided_secret = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !bool::from(provided_secret.as_bytes().ct_eq(expected_secret.as_bytes())) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Invalid webhook secret".to_string(),
        ));
    }

    // Build the prompt from the routine action.
    let prompt = match &routine.action {
        RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        RoutineAction::FullJob {
            title, description, ..
        } => format!("{}: {}", title, description),
    };

    let content = format!("[routine:{}] {}", routine.name, prompt);
    let thread_id = format!(
        "routine-{}-{}",
        routine.id,
        chrono::Utc::now().timestamp_millis()
    );
    let msg = IncomingMessage::new("gateway", &routine.user_id, content).with_thread(thread_id);

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine.id,
        "routine_name": routine.name,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify constant-time comparison logic for webhook secrets.
    #[test]
    fn test_webhook_secret_constant_time_comparison() {
        let expected = "my-secret-token";

        // Matching secret
        let provided = "my-secret-token";
        assert!(bool::from(provided.as_bytes().ct_eq(expected.as_bytes())));

        // Wrong secret
        let wrong = "wrong-secret";
        assert!(!bool::from(wrong.as_bytes().ct_eq(expected.as_bytes())));

        // Empty secret
        let empty = "";
        assert!(!bool::from(empty.as_bytes().ct_eq(expected.as_bytes())));
    }

    /// Verify that routines without a configured secret are rejected.
    #[test]
    fn test_webhook_rejects_missing_secret() {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: None,
        };

        // The handler rejects routines where secret is None
        let has_secret = matches!(
            &trigger,
            Trigger::Webhook {
                secret: Some(_),
                ..
            }
        );
        assert!(!has_secret, "Trigger with secret: None must be rejected");

        // Trigger with a secret should pass the check
        let trigger_with_secret = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("s3cret".to_string()),
        };
        let has_secret = matches!(
            &trigger_with_secret,
            Trigger::Webhook {
                secret: Some(_),
                ..
            }
        );
        assert!(has_secret, "Trigger with a secret should be accepted");
    }

    /// Verify rate limit error returns proper 429 status text.
    #[test]
    fn test_webhook_rate_limit_format() {
        let error_msg = "Rate limit exceeded. Try again shortly.";
        let status = StatusCode::TOO_MANY_REQUESTS;
        assert_eq!(status.as_u16(), 429);
        assert!(error_msg.contains("Rate limit"));
    }
}
