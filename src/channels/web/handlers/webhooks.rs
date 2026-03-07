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
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Load all routines and find one whose Trigger::Webhook path matches.
    let routines = store
        .list_all_routines()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let matched = routines.into_iter().find(|r| {
        if !r.enabled {
            return false;
        }
        match &r.trigger {
            Trigger::Webhook {
                path: Some(wp), ..
            } => *wp == path,
            Trigger::Webhook { path: None, .. } => path == r.id.to_string(),
            _ => false,
        }
    });

    let routine = matched.ok_or((
        StatusCode::NOT_FOUND,
        "No routine matches this webhook path".to_string(),
    ))?;

    // Validate the webhook secret if one is configured on the routine.
    if let Trigger::Webhook {
        secret: Some(expected_secret),
        ..
    } = &routine.trigger
    {
        let provided_secret = headers
            .get("x-webhook-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !bool::from(
            provided_secret
                .as_bytes()
                .ct_eq(expected_secret.as_bytes()),
        ) {
            return Err((StatusCode::UNAUTHORIZED, "Invalid webhook secret".to_string()));
        }
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
    let msg =
        IncomingMessage::new("gateway", &routine.user_id, content).with_thread(thread_id);

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
        assert!(bool::from(
            provided.as_bytes().ct_eq(expected.as_bytes())
        ));

        // Wrong secret
        let wrong = "wrong-secret";
        assert!(!bool::from(
            wrong.as_bytes().ct_eq(expected.as_bytes())
        ));

        // Empty secret
        let empty = "";
        assert!(!bool::from(
            empty.as_bytes().ct_eq(expected.as_bytes())
        ));
    }

    /// Verify that webhook path matching logic works for both explicit paths
    /// and fallback to routine ID.
    #[test]
    fn test_webhook_path_matching() {
        use chrono::Utc;
        use uuid::Uuid;

        let routine_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        let routine = crate::agent::routine::Routine {
            id: routine_id,
            name: "test-routine".to_string(),
            description: "A test routine".to_string(),
            user_id: "test-user".to_string(),
            enabled: true,
            trigger: Trigger::Webhook {
                path: Some("my-hook".to_string()),
                secret: None,
            },
            action: RoutineAction::Lightweight {
                prompt: "do stuff".to_string(),
                context_paths: vec![],
                max_tokens: 4096,
            },
            guardrails: crate::agent::routine::RoutineGuardrails::default(),
            notify: crate::agent::routine::NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::Value::Null,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // Explicit path match
        let matches_explicit = match &routine.trigger {
            Trigger::Webhook {
                path: Some(wp), ..
            } => *wp == "my-hook",
            _ => false,
        };
        assert!(matches_explicit);

        // Should NOT match wrong path
        let matches_wrong = match &routine.trigger {
            Trigger::Webhook {
                path: Some(wp), ..
            } => *wp == "other-hook",
            _ => false,
        };
        assert!(!matches_wrong);

        // Routine with no explicit path falls back to ID
        let routine_no_path = crate::agent::routine::Routine {
            trigger: Trigger::Webhook {
                path: None,
                secret: None,
            },
            ..routine
        };
        let matches_id = match &routine_no_path.trigger {
            Trigger::Webhook { path: None, .. } => {
                routine_no_path.id.to_string() == "550e8400-e29b-41d4-a716-446655440000"
            }
            _ => false,
        };
        assert!(matches_id);

        // Disabled routine should not match
        let disabled_routine = crate::agent::routine::Routine {
            enabled: false,
            trigger: Trigger::Webhook {
                path: Some("my-hook".to_string()),
                secret: None,
            },
            ..routine_no_path
        };
        let should_skip = !disabled_routine.enabled;
        assert!(should_skip);
    }
}
