/// Test double substituting the bottom-of-stack `NetworkHttpTransport` (the
/// would-be socket) underneath the REAL production `PolicyNetworkHttpEgress`
/// (`crates/ironclaw_network/src/egress.rs`). Real network-policy enforcement
/// (allowlist/scheme/port, private-IP denial) and DNS resolution run before
/// this transport is ever reached — it only sees requests that already
/// cleared policy. Distinct from `RecordingNetworkHttpEgress`, which
/// implements `NetworkHttpEgress` directly and so bypasses that policy layer
/// entirely.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use ironclaw_network::{
    DEFAULT_RESPONSE_BODY_LIMIT, NetworkHttpError, NetworkHttpResponse, NetworkHttpTransport,
    NetworkTransportRequest, NetworkUsage,
};

#[derive(Debug, Clone)]
pub(crate) struct RecordingNetworkHttpTransport {
    inner: Arc<Mutex<RecordingNetworkTransportState>>,
}

#[derive(Debug, Default)]
struct RecordingNetworkTransportState {
    default_body: Vec<u8>,
    response_bodies: VecDeque<Vec<u8>>,
    requests: Vec<NetworkTransportRequest>,
}

impl RecordingNetworkHttpTransport {
    pub(crate) fn with_body(body: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordingNetworkTransportState {
                default_body: body,
                response_bodies: VecDeque::new(),
                requests: Vec::new(),
            })),
        }
    }

    /// Every request that reached this transport, in call order — i.e. every
    /// request that survived real network-policy enforcement and DNS
    /// resolution above it. Empty means policy denied the call before it got
    /// this far.
    pub(crate) fn requests(&self) -> Vec<NetworkTransportRequest> {
        self.inner.lock().unwrap().requests.clone()
    }

    /// Enqueue one FIFO scripted response body, consumed ahead of the default
    /// body — lets a test script a response containing a seeded secret for
    /// the real leak-scan pipeline to react to.
    pub(crate) fn push_response_body(&self, body: Vec<u8>) {
        self.inner.lock().unwrap().response_bodies.push_back(body);
    }
}

#[async_trait::async_trait]
impl NetworkHttpTransport for RecordingNetworkHttpTransport {
    async fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let mut state = self.inner.lock().unwrap();
        let request_bytes = request.body.len() as u64;
        state.requests.push(request.clone());
        let mut body = state
            .response_bodies
            .pop_front()
            .unwrap_or_else(|| state.default_body.clone());
        let headers = vec![("content-type".to_string(), "application/json".to_string())];
        // Mirror `ReqwestNetworkTransport`: a body over the effective response
        // limit is truncated to the limit and surfaced as a `ResponseBodyLimit`
        // error carrying the partial response.
        let limit = request
            .response_body_limit
            .unwrap_or(DEFAULT_RESPONSE_BODY_LIMIT)
            .min(DEFAULT_RESPONSE_BODY_LIMIT);
        if body.len() as u64 > limit {
            body.truncate(limit as usize);
            let response_bytes = limit.saturating_add(1);
            return Err(NetworkHttpError::ResponseBodyLimit {
                limit,
                request_bytes,
                response_bytes,
                partial_response: Some(NetworkHttpResponse {
                    status: 200,
                    headers,
                    body,
                    usage: NetworkUsage {
                        request_bytes,
                        response_bytes,
                        resolved_ip: None,
                    },
                }),
            });
        }
        Ok(NetworkHttpResponse {
            status: 200,
            headers,
            body: body.clone(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: body.len() as u64,
                resolved_ip: None,
            },
        })
    }
}
