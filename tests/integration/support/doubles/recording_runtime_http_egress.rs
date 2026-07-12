/// Test double substituting the production `RuntimeHttpEgress` impl
/// (`HostHttpEgressService`, `crates/ironclaw_host_runtime/src/egress/mod.rs`).
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
};

#[derive(Debug, Clone)]
pub(crate) struct RecordingRuntimeHttpEgress {
    default_body: Vec<u8>,
    /// URL/method/capability-keyed scripted responses (§3.6 P1 ergonomics).
    /// Consulted before the FIFO queue; first match wins.
    scripted: Arc<Mutex<Vec<super::super::http_matcher::ScriptedHttpResponse>>>,
    response_bodies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeHttpEgress {
    pub(crate) fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            scripted: Arc::new(Mutex::new(Vec::new())),
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Append keyed scripted responses (the canonical keyed-matcher install).
    pub(crate) fn install_scripted(
        &self,
        responses: impl IntoIterator<Item = super::super::http_matcher::ScriptedHttpResponse>,
    ) {
        self.scripted.lock().unwrap().extend(responses);
    }

    /// Enqueue one FIFO response body (C-WEBACCESS), consumed in call order
    /// ahead of `default_body`. Mirrors `install_scripted`'s shape but for the
    /// plain FIFO queue rather than keyed matchers — used to script the
    /// three-leg Exa MCP handshake (`initialize` → `notifications/initialized`
    /// → `tools/call`), which all target the same URL/method/capability and so
    /// cannot be told apart by the keyed matcher.
    pub(crate) fn push_response_body(&self, body: Vec<u8>) {
        self.response_bodies.lock().unwrap().push_back(body);
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let request_bytes = request.body.len() as u64;
        // Resolve the keyed outcome BEFORE recording the request: `push(request)`
        // moves `request` by value into the log, so any code reading its fields
        // (the `.matches()` lookup) must run first. (`RuntimeHttpEgressRequest`
        // does implement `Drop`/`ZeroizeOnDrop` to scrub its URL/headers, but that
        // fires later when the logged entry is actually dropped, not on push.)
        let keyed_outcome = {
            let scripted = self.scripted.lock().unwrap();
            scripted
                .iter()
                .find(|response| response.matches(&request))
                .map(|response| response.outcome())
        };
        self.requests.lock().unwrap().push(request);
        // A scripted egress error short-circuits with `Err`, driving the tool's
        // error mapping. A body outcome (or the FIFO/default fallback) returns
        // `Ok` with the scripted status/body.
        let (status, body) = match keyed_outcome {
            Some(super::super::http_matcher::ScriptedHttpOutcome::Error(error)) => {
                return Err(error);
            }
            Some(super::super::http_matcher::ScriptedHttpOutcome::Body { status, bytes }) => {
                (status, bytes)
            }
            None => (
                200,
                self.response_bodies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| self.default_body.clone()),
            ),
        };
        Ok(RuntimeHttpEgressResponse {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: body.clone(),
            saved_body: None,
            request_bytes,
            response_bytes: body.len() as u64,
            redaction_applied: false,
        })
    }
}

#[async_trait]
impl ironclaw_host_runtime::ToolCallHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute_for_model_visible_output(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        RuntimeHttpEgress::execute(self, request).await
    }
}
