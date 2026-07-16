//! Run-cancellation mutation.
//!
//! Wire route: `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel`
//! (`ironclaw_webui_v2::handlers::cancel_run`). Body shape is
//! `WebUiCancelRunRequest` (`ironclaw_product_workflow`); path `thread_id`/
//! `run_id` override any body values (see the handler's own doc comment),
//! so this method leaves them unset and sends an otherwise-empty body,
//! mirroring `client/gates.rs`'s `resolve_gate`.

use ironclaw_product_workflow::WebUiCancelRunRequest;
use uuid::Uuid;

use super::{ApiClient, ClientError};

impl ApiClient {
    pub async fn cancel_run(&self, thread_id: &str, run_id: &str) -> Result<(), ClientError> {
        let body = WebUiCancelRunRequest {
            client_action_id: Some(Uuid::new_v4().to_string()),
            ..Default::default()
        };
        self.send_unit(
            self.http
                .post(self.url(&format!(
                    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel"
                )))
                .json(&body),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::{OriginalUri, State};
    use axum::http::Method;
    use axum::routing::post;
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Option<(Method, String)>>>);

    async fn cancel_run_stub(
        State(captured): State<Captured>,
        method: Method,
        OriginalUri(uri): OriginalUri,
    ) -> axum::Json<serde_json::Value> {
        *captured.0.lock().expect("lock captured request") = Some((method, uri.path().to_string()));
        axum::Json(serde_json::json!({
            "run_id": "run-1",
            "status": "cancelled",
            "event_cursor": 1,
            "already_terminal": false,
        }))
    }

    /// Asserts `cancel_run` hits the exact existing server route (path
    /// `thread_id`/`run_id` interpolated, `POST` method) rather than just
    /// that the client method returns `Ok`.
    #[tokio::test]
    async fn cancel_run_posts_to_the_thread_run_cancel_route() {
        let captured = Captured::default();
        let router = Router::new()
            .route(
                "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
                post(cancel_run_stub),
            )
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub listener");
        let addr = listener.local_addr().expect("stub listener addr");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("stub server serve");
        });

        let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
        client
            .cancel_run("t-1", "run-1")
            .await
            .expect("cancel_run succeeds against stub");

        let (method, path) = captured
            .0
            .lock()
            .expect("lock captured request")
            .clone()
            .expect("stub must have been hit");
        assert_eq!(method, Method::POST);
        assert_eq!(path, "/api/webchat/v2/threads/t-1/runs/run-1/cancel");
    }
}
