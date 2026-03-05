//! Routine management API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

pub async fn routines_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_all_routines()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<RoutineInfo> = routines.iter().map(routine_to_info).collect();

    Ok(Json(RoutineListResponse { routines: items }))
}

pub async fn routines_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_all_routines()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = routines.len() as u64;
    let enabled = routines.iter().filter(|r| r.enabled).count() as u64;
    let disabled = total - enabled;
    let failing = routines
        .iter()
        .filter(|r| r.consecutive_failures > 0)
        .count() as u64;

    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc());
    let runs_today = if let Some(start) = today_start {
        routines
            .iter()
            .filter(|r| r.last_run_at.is_some_and(|ts| ts >= start))
            .count() as u64
    } else {
        0
    };

    Ok(Json(RoutineSummaryResponse {
        total,
        enabled,
        disabled,
        failing,
        runs_today,
    }))
}

pub async fn routines_detail_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<RoutineDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    let runs = store
        .list_routine_runs(routine_id, 20)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let recent_runs: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(RoutineDetailResponse {
        id: routine.id,
        name: routine.name.clone(),
        description: routine.description.clone(),
        enabled: routine.enabled,
        trigger: serde_json::to_value(&routine.trigger).unwrap_or_default(),
        action: serde_json::to_value(&routine.action).unwrap_or_default(),
        guardrails: serde_json::to_value(&routine.guardrails).unwrap_or_default(),
        notify: serde_json::to_value(&routine.notify).unwrap_or_default(),
        last_run_at: routine.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: routine.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: routine.run_count,
        consecutive_failures: routine.consecutive_failures,
        created_at: routine.created_at.to_rfc3339(),
        recent_runs,
    }))
}

pub async fn routines_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    if routine.user_id != state.user_id {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    // Send the routine prompt through the message pipeline as a manual trigger.
    let prompt = match &routine.action {
        crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        crate::agent::routine::RoutineAction::FullJob {
            title, description, ..
        } => format!("{}: {}", title, description),
    };

    let content = format!("[routine:{}] {}", routine.name, prompt);
    let thread_id = format!(
        "routine-{}-{}",
        routine_id,
        chrono::Utc::now().timestamp_millis()
    );
    let msg = IncomingMessage::new("gateway", &state.user_id, content).with_thread(thread_id);

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
        "routine_id": routine_id,
    })))
}

#[derive(Deserialize)]
pub struct ToggleRequest {
    pub enabled: Option<bool>,
}

pub async fn routines_toggle_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    body: Option<Json<ToggleRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let mut routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    // If a specific value was provided, use it; otherwise toggle.
    routine.enabled = match body {
        Some(Json(req)) => req.enabled.unwrap_or(!routine.enabled),
        None => !routine.enabled,
    };

    store
        .update_routine(&routine)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": if routine.enabled { "enabled" } else { "disabled" },
        "routine_id": routine_id,
    })))
}

pub async fn routines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let deleted = store
        .delete_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        Ok(Json(serde_json::json!({
            "status": "deleted",
            "routine_id": routine_id,
        })))
    } else {
        Err((StatusCode::NOT_FOUND, "Routine not found".to_string()))
    }
}

pub async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let runs = store
        .list_routine_runs(routine_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let run_infos: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(serde_json::json!({
        "routine_id": routine_id,
        "runs": run_infos,
    })))
}

/// Convert a Routine to the trimmed RoutineInfo for list display.
fn routine_to_info(r: &crate::agent::routine::Routine) -> RoutineInfo {
    let (trigger_type, trigger_summary) = match &r.trigger {
        crate::agent::routine::Trigger::Cron { schedule } => {
            ("cron".to_string(), format!("cron: {}", schedule))
        }
        crate::agent::routine::Trigger::Event {
            pattern, channel, ..
        } => {
            let ch = channel.as_deref().unwrap_or("any");
            ("event".to_string(), format!("on {} /{}/", ch, pattern))
        }
        crate::agent::routine::Trigger::Webhook { path, .. } => {
            let p = path.as_deref().unwrap_or("/");
            ("webhook".to_string(), format!("webhook: {}", p))
        }
        crate::agent::routine::Trigger::Manual => ("manual".to_string(), "manual only".to_string()),
    };

    let action_type = match &r.action {
        crate::agent::routine::RoutineAction::Lightweight { .. } => "lightweight",
        crate::agent::routine::RoutineAction::FullJob { .. } => "full_job",
    };

    let status = if !r.enabled {
        "disabled"
    } else if r.consecutive_failures > 0 {
        "failing"
    } else {
        "active"
    };

    RoutineInfo {
        id: r.id,
        name: r.name.clone(),
        description: r.description.clone(),
        enabled: r.enabled,
        trigger_type,
        trigger_summary,
        action_type: action_type.to_string(),
        last_run_at: r.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: r.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: r.run_count,
        consecutive_failures: r.consecutive_failures,
        status: status.to_string(),
    }
}

pub async fn webhook_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(identifier): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Try to parse identifier as UUID first
    let routine = if let Ok(routine_id) = Uuid::parse_str(&identifier) {
        store.get_routine(routine_id).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
        })?
    } else {
        // If it's not a UUID, currently we don't have a get_routine_by_path in Store,
        // so we reject it. To fully support string paths, we would need to add
        // get_routine_by_webhook_path to the Database trait.
        // For now, we only support UUID identifiers in the webhook URL.
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid routine ID format (expected UUID)".to_string(),
        ));
    };

    let routine = routine.ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    if !routine.enabled {
        return Err((StatusCode::BAD_REQUEST, "Routine is disabled".to_string()));
    }

    // Verify it is a webhook trigger
    let trigger_secret = match &routine.trigger {
        crate::agent::routine::Trigger::Webhook { secret, .. } => secret.clone(),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Routine is not configured for webhook triggers".to_string(),
            ));
        }
    };

    // If a secret is configured, perform HMAC validation
    if let Some(secret) = trigger_secret {
        if secret.is_empty() {
            // Secret exists but is empty, allow (though not recommended)
        } else {
            // Check for common signature headers
            // e.g. X-Hub-Signature-256 for GitHub, Stripe-Signature for Stripe
            let signature_header = headers
                .get("x-hub-signature-256")
                .or_else(|| headers.get("stripe-signature"))
                .or_else(|| headers.get("x-webhook-signature"))
                .and_then(|v| v.to_str().ok());

            let signature = match signature_header {
                Some(sig) => sig,
                None => {
                    tracing::warn!(routine = %routine.name, "Webhook missing signature header");
                    return Err((
                        StatusCode::UNAUTHORIZED,
                        "Missing webhook signature header".to_string(),
                    ));
                }
            };

            // Extract the actual hex/b64 value depending on the provider format
            // GitHub format: "sha256=HEX..."
            let sig_value = if let Some(stripped) = signature.strip_prefix("sha256=") {
                stripped
            } else {
                signature
            };

            use hmac::{Hmac, Mac};
            use sha2::Sha256;

            type HmacSha256 = Hmac<Sha256>;

            let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Invalid secret key length".to_string(),
                )
            })?;

            mac.update(&body);

            if let Ok(expected_mac) = hex::decode(sig_value) {
                if mac.verify_slice(&expected_mac).is_err() {
                    tracing::warn!(routine = %routine.name, "Webhook signature verification failed");
                    return Err((
                        StatusCode::UNAUTHORIZED,
                        "Invalid webhook signature".to_string(),
                    ));
                }
            } else {
                tracing::warn!(routine = %routine.name, "Webhook signature is not valid hex");
                return Err((
                    StatusCode::UNAUTHORIZED,
                    "Invalid webhook signature format".to_string(),
                ));
            }
        }
    }

    // Try to parse the payload to extract a summary or pass it along
    let payload_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => "[Binary payload]".to_string(),
    };

    let prompt = match &routine.action {
        crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        crate::agent::routine::RoutineAction::FullJob {
            title, description, ..
        } => format!("{}: {}", title, description),
    };

    // Include the payload in the message to the agent so it knows *why* it was triggered
    let content = format!(
        "[routine:{}]\n{}\n\nWebhook Payload:\n```json\n{}\n```",
        routine.name, prompt, payload_str
    );

    let thread_id = format!(
        "routine-{}-{}",
        routine.id,
        chrono::Utc::now().timestamp_millis()
    );

    // We impersonate the routine's owner to ensure it runs with correct permissions
    let msg = IncomingMessage::new("webhook", &routine.user_id, content).with_thread(thread_id);

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

    tracing::info!(routine = %routine.name, "Triggered via webhook");
    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine.id,
    })))
}

#[cfg(test)]
mod tests {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // We mock the handler logic since testing axum routers end-to-end requires setup.
    // However, we verify the HMAC generation and validation logic here.

    #[test]
    fn test_valid_webhook_hmac() {
        let secret = "super-secret-key-123";
        let body = b"{\"event\": \"push\", \"ref\": \"refs/heads/main\"}";

        // Generate HMAC signature as the "external service" would
        type HmacSha256 = Hmac<Sha256>;
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(body);
        let valid_signature = hex::encode(mac.finalize().into_bytes());

        // Now simulate the handler receiving it
        let mut handler_mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        handler_mac.update(body);

        let expected_mac = hex::decode(&valid_signature).unwrap();

        let result = handler_mac.verify_slice(&expected_mac);
        assert!(
            result.is_ok(),
            "Valid signature should pass HMAC verification"
        );
    }

    #[test]
    fn test_invalid_webhook_hmac() {
        let secret = "super-secret-key-123";
        let wrong_secret = "wrong-secret-key-123";
        let body = b"{\"event\": \"push\", \"ref\": \"refs/heads/main\"}";

        type HmacSha256 = Hmac<Sha256>;

        // Generate with wrong secret
        let mut wrong_mac = HmacSha256::new_from_slice(wrong_secret.as_bytes()).unwrap();
        wrong_mac.update(body);
        let invalid_signature = hex::encode(wrong_mac.finalize().into_bytes());

        // Validate with correct secret
        let mut handler_mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        handler_mac.update(body);

        let expected_mac = hex::decode(&invalid_signature).unwrap();

        let result = handler_mac.verify_slice(&expected_mac);
        assert!(
            result.is_err(),
            "Invalid signature should fail HMAC verification"
        );
    }
}
