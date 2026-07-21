/// Test double substituting the production `NetworkHttpEgress` impl:
/// `PolicyNetworkHttpEgress` (`crates/ironclaw_network/src/egress.rs`) over
/// `ReqwestNetworkTransport` (`crates/ironclaw_network/src/transport.rs`).
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};

#[derive(Debug, Clone)]
pub(crate) struct RecordingNetworkHttpEgress {
    default_body: Vec<u8>,
    scripted_responses: Arc<Mutex<VecDeque<ScriptedNetworkResponse>>>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

#[derive(Debug)]
enum ScriptedNetworkResponse {
    Status(u16),
    Complete { status: u16, body: Vec<u8> },
}

impl RecordingNetworkHttpEgress {
    pub(crate) fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            scripted_responses: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Enqueue one FIFO scripted status, consumed by the next `execute` call
    /// ahead of the hardcoded `200` default.
    pub(crate) fn push_status(&self, status: u16) {
        self.scripted_responses
            .lock()
            .unwrap()
            .push_back(ScriptedNetworkResponse::Status(status));
    }

    /// Enqueue one complete FIFO response. Status and body are consumed by the
    /// same next request, allowing guest-runtime tests to exercise exact HTTP
    /// error classification before the default success response resumes.
    pub(crate) fn push_response(&self, status: u16, body: Vec<u8>) {
        self.scripted_responses
            .lock()
            .unwrap()
            .push_back(ScriptedNetworkResponse::Complete { status, body });
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for RecordingNetworkHttpEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        self.requests.lock().unwrap().push(request);
        let (status, body) = match self.scripted_responses.lock().unwrap().pop_front() {
            Some(ScriptedNetworkResponse::Status(status)) => (status, self.default_body.clone()),
            Some(ScriptedNetworkResponse::Complete { status, body }) => (status, body),
            None => (200, self.default_body.clone()),
        };
        Ok(NetworkHttpResponse {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: body.clone(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: body.len() as u64,
                resolved_ip: None,
            },
        })
    }
}
