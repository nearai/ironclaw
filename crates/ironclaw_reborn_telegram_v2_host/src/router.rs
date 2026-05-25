//! Axum webhook route for the Reborn Telegram v2 host.
//!
//! Pattern: `POST /webhook/telegram-v2/{installation_id}` — delegates the
//! whole webhook lifecycle to [`NativeProductAdapterRunner::process_webhook`]
//! which handles auth, parsing, workflow dispatch, and ack mapping.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ironclaw_wasm_product_adapters::{
    NativeProductAdapterRunner, RunnerError, WebhookProcessOutcome,
};

#[derive(Clone)]
pub struct TelegramV2RouterState {
    pub runners: Arc<HashMap<String, Arc<NativeProductAdapterRunner>>>,
}

pub fn telegram_v2_routes(state: TelegramV2RouterState) -> Router {
    Router::new()
        .route("/webhook/telegram-v2/{installation_id}", post(handle))
        .with_state(state)
}

async fn handle(
    State(state): State<TelegramV2RouterState>,
    Path(installation_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(runner) = state.runners.get(&installation_id) else {
        return (
            StatusCode::NOT_FOUND,
            format!("unknown telegram v2 installation: {installation_id}"),
        )
            .into_response();
    };

    match runner.process_webhook(&headers, &body).await {
        // Upstream consolidated the prior `NoOp` outcome into `Acknowledged`
        // with a no-op ack — `ProductInboundPayload::NoOp` events still flow
        // through `ProductWorkflow` and ack 200.
        Ok(WebhookProcessOutcome::Acknowledged { .. }) => StatusCode::OK.into_response(),
        Err(RunnerError::AuthenticationFailed { failure }) => {
            tracing::warn!(
                installation = %installation_id,
                failure = ?failure,
                "telegram v2 webhook auth failed"
            );
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(RunnerError::TooManyInFlight { max_in_flight }) => {
            tracing::warn!(
                installation = %installation_id,
                max_in_flight,
                "telegram v2 webhook throttled"
            );
            StatusCode::TOO_MANY_REQUESTS.into_response()
        }
        Err(err) => {
            tracing::error!(
                installation = %installation_id,
                error = %err,
                "telegram v2 webhook processing failed"
            );
            if err.is_retryable() {
                StatusCode::SERVICE_UNAVAILABLE.into_response()
            } else {
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}
