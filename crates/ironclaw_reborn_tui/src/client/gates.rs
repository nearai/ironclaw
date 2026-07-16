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
use uuid::Uuid;

use super::{ApiClient, ClientError};

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
}
