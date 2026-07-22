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

/// Request-keyed scripted response: consulted before the FIFO/default lanes
/// so vendor-shaped fixtures (Slack `chat.postMessage` vs Telegram
/// `sendMessage`) can answer by URL regardless of call order.
pub(crate) type VendorResponseRouter =
    dyn Fn(&NetworkHttpRequest) -> Option<(u16, Vec<u8>)> + Send + Sync;

#[derive(Clone)]
pub(crate) struct RecordingNetworkHttpEgress {
    default_body: Vec<u8>,
    response_bodies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    /// W4-AUTHGATE-WIRE: FIFO of scripted non-default statuses, consumed ahead
    /// of the hardcoded `200` default. Lets a test drive the runtime-401 path
    /// for capabilities whose real HTTP call flows through this **network**
    /// lane (`GithubIssueTools`, via `try_with_host_http_egress`) rather than
    /// the runtime egress matcher. Empty by default — pre-existing callers
    /// keep the old hardcoded-200 behavior byte-identical.
    status_queue: Arc<Mutex<VecDeque<u16>>>,
    vendor_router: Option<Arc<VendorResponseRouter>>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl std::fmt::Debug for RecordingNetworkHttpEgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingNetworkHttpEgress")
            .field("recorded_requests", &self.requests.lock().unwrap().len())
            .field("has_vendor_router", &self.vendor_router.is_some())
            .finish()
    }
}

impl RecordingNetworkHttpEgress {
    pub(crate) fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            status_queue: Arc::new(Mutex::new(VecDeque::new())),
            vendor_router: None,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Attach a request-keyed vendor router, consulted before the FIFO
    /// queues and the fixed default body.
    pub(crate) fn with_vendor_router(mut self, router: Arc<VendorResponseRouter>) -> Self {
        self.vendor_router = Some(router);
        self
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
        let routed = self
            .vendor_router
            .as_ref()
            .and_then(|router| router(&request));
        self.requests.lock().unwrap().push(request);
        let (status, body) = match routed {
            Some((status, body)) => (status, body),
            None => (
                self.status_queue.lock().unwrap().pop_front().unwrap_or(200),
                self.response_bodies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| self.default_body.clone()),
            ),
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
