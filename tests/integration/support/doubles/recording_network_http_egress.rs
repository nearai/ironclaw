/// Test double substituting the production `NetworkHttpEgress` impl
/// (`PolicyNetworkHttpEgress` over `ReqwestNetworkTransport`,
/// `crates/ironclaw_network/src/egress.rs`).
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
    response_bodies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    /// W4-AUTHGATE-WIRE: FIFO of scripted non-default statuses, consumed ahead
    /// of the hardcoded `200` default. Lets a test drive the runtime-401
    /// (credential-injected-but-rejected) path for capabilities whose real
    /// HTTP call flows through this **network** lane rather than the runtime
    /// egress `ScriptedHttpResponse` matcher (`GithubIssueTools` — see
    /// `reborn_integration_secret_injection.rs`'s module doc: `try_with_host_http_egress`
    /// overwrites the runtime port with the host pipeline over THIS recorder).
    /// Empty by default — every pre-existing caller keeps the old hardcoded-200
    /// behavior byte-identical.
    status_queue: Arc<Mutex<VecDeque<u16>>>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetworkHttpEgress {
    pub(crate) fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            status_queue: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Enqueue one FIFO scripted status, consumed by the next `execute` call
    /// ahead of the hardcoded `200` default.
    pub(crate) fn push_status(&self, status: u16) {
        self.status_queue.lock().unwrap().push_back(status);
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
        let body = self
            .response_bodies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.default_body.clone());
        let status = self.status_queue.lock().unwrap().pop_front().unwrap_or(200);
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
