//! Send-message + gate-resolution mutations.
//!
//! Per review-round-2 override #1: no local `GateResolution`/
//! `ResolveGateBody`/message-body types — reuse
//! `ironclaw_product_workflow`'s `WebUiGateResolution` (the client-facing
//! resolution enum), `WebUiResolveGateRequest`, and `WebUiSendMessageRequest`
//! directly. `WebUiResolveGateRequest` flattens `resolution` to a plain
//! string field rather than a tagged enum, so [`resolve_gate`] maps each
//! `WebUiGateResolution` variant onto it by hand:
//! `Approved{always}` -> `resolution: Some("approved")` + `always`;
//! `Declined` -> `resolution: Some("denied")` (not `"declined"` —
//! `parse_gate_resolution` in `webui_inbound.rs` only recognizes
//! `"denied"`/`"cancelled"` for this variant, matching the pre-existing
//! wire contract); `CredentialProvided{credential_ref}` ->
//! `resolution: Some("credential_provided")` + `credential_ref`.

use ironclaw_product_workflow::{
    WebUiGateResolution, WebUiResolveGateRequest, WebUiSendMessageRequest,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ApiClient, ClientError};

/// Wire route: `POST /api/reborn/product-auth/manual-token/submit`
/// (`ironclaw_reborn_composition::product_auth::serve::manual_token`). That
/// route's `ManualTokenSubmitRequest`/`ManualTokenSubmitResponse` types are
/// `pub(super)`/`pub(crate)` to `ironclaw_reborn_composition` — not exported
/// from any crate this one may depend on (see `app/mod.rs`'s dependency
/// boundary doc) — so this is a local, wire-shape-matching mirror rather
/// than a shared type. Field names/shape lifted from
/// `crates/ironclaw_webui_v2/frontend/src/lib/api.ts`'s `submitManualToken`,
/// the browser client sending the same request.
#[derive(Debug, Serialize)]
struct ManualTokenSubmitBody {
    provider: String,
    account_label: String,
    token: String,
    run_id: String,
    gate_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
}

/// Only the field this client needs (`credential_ref`, the UUID fed back
/// into [`ApiClient::resolve_gate`]'s `CredentialProvided` resolution) —
/// `status`/`continuation` on the real server response are typed with enums
/// this crate cannot name, and serde ignores unrecognized fields by default,
/// so leaving them off here is safe.
#[derive(Debug, Deserialize)]
struct ManualTokenSubmitResponse {
    credential_ref: String,
}

fn resolve_gate_body(resolution: WebUiGateResolution) -> WebUiResolveGateRequest {
    let (resolution, always, credential_ref) = match resolution {
        WebUiGateResolution::Approved { always } => ("approved", Some(always), None),
        WebUiGateResolution::Declined => ("denied", None, None),
        WebUiGateResolution::CredentialProvided { credential_ref } => {
            ("credential_provided", None, Some(credential_ref))
        }
    };
    WebUiResolveGateRequest {
        client_action_id: Some(Uuid::new_v4().to_string()),
        resolution: Some(resolution.to_string()),
        always,
        credential_ref,
        ..Default::default()
    }
}

impl ApiClient {
    /// Ack for the submitted message arrives via the SSE stream
    /// (`WebChatV2Event::Accepted`), not this response — the interface
    /// contract's "(ack arrives via stream)" note.
    pub async fn send_message(&self, thread_id: &str, text: &str) -> Result<(), ClientError> {
        let body = WebUiSendMessageRequest {
            client_action_id: Some(Uuid::new_v4().to_string()),
            content: Some(text.to_string()),
            ..Default::default()
        };
        self.send_unit(
            self.http
                .post(self.url(&format!("/api/webchat/v2/threads/{thread_id}/messages")))
                .json(&body),
        )
        .await
    }

    pub async fn resolve_gate(
        &self,
        thread_id: &str,
        run_id: &str,
        gate_ref: &str,
        resolution: WebUiGateResolution,
    ) -> Result<(), ClientError> {
        let body = resolve_gate_body(resolution);
        self.send_unit(
            self.http
                .post(self.url(&format!(
                    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve"
                )))
                .json(&body),
        )
        .await
    }

    /// Step 1 of the manual-token auth flow: stores the raw token
    /// server-side and returns a `credential_ref` (a UUID) the caller then
    /// feeds into [`Self::resolve_gate`]'s `CredentialProvided` resolution
    /// (step 2) — the server's `parse_credential_account_id` requires a
    /// UUID, not the raw token. See this module's doc comment for why the
    /// request/response shapes are local mirrors.
    pub async fn submit_manual_token(
        &self,
        provider: &str,
        account_label: &str,
        token: &str,
        thread_id: &str,
        run_id: &str,
        gate_ref: &str,
    ) -> Result<String, ClientError> {
        let body = ManualTokenSubmitBody {
            provider: provider.to_string(),
            account_label: account_label.to_string(),
            token: token.to_string(),
            run_id: run_id.to_string(),
            gate_ref: gate_ref.to_string(),
            thread_id: Some(thread_id.to_string()),
        };
        let response: ManualTokenSubmitResponse = self
            .send_json(
                self.http
                    .post(self.url("/api/reborn/product-auth/manual-token/submit"))
                    .json(&body),
            )
            .await?;
        Ok(response.credential_ref)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::{Json as JsonExtract, State};
    use axum::routing::post;
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Option<serde_json::Value>>>);

    async fn manual_token_submit_stub(
        State(captured): State<Captured>,
        JsonExtract(body): JsonExtract<serde_json::Value>,
    ) -> axum::Json<serde_json::Value> {
        *captured.0.lock().expect("lock captured body") = Some(body);
        axum::Json(serde_json::json!({
            "credential_ref": "cred-11111111-1111-1111-1111-111111111111",
            "status": "active",
            "continuation": "resumed",
        }))
    }

    /// Pins the request path and the exact body shape (`token` alongside
    /// `provider`/`account_label`/`run_id`/`gate_ref`/`thread_id`) webui's
    /// `submitManualToken` sends, and that the UUID-shaped `credential_ref`
    /// comes back out — not just that the call returns `Ok`.
    #[tokio::test]
    async fn submit_manual_token_posts_the_raw_token_and_returns_credential_ref() {
        let captured = Captured::default();
        let router = Router::new()
            .route(
                "/api/reborn/product-auth/manual-token/submit",
                post(manual_token_submit_stub),
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
        let credential_ref = client
            .submit_manual_token(
                "google",
                "work@example.com",
                "raw-secret",
                "t-1",
                "run-1",
                "gate-1",
            )
            .await
            .expect("submit_manual_token succeeds against stub");

        assert_eq!(credential_ref, "cred-11111111-1111-1111-1111-111111111111");
        let body = captured
            .0
            .lock()
            .expect("lock captured body")
            .clone()
            .expect("stub must have been hit");
        assert_eq!(body["provider"], "google");
        assert_eq!(body["account_label"], "work@example.com");
        assert_eq!(body["token"], "raw-secret");
        assert_eq!(body["run_id"], "run-1");
        assert_eq!(body["gate_ref"], "gate-1");
        assert_eq!(body["thread_id"], "t-1");
    }
}
